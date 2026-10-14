//! Source Drop protocol selection and reference-receiver contracts.
#[test]
fn cmtt_quote_construction_is_inert_and_each_splice_cleans_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        fn entry()->bool {
            const unused='(make());
            const code='(do {let item=make();true});
            ~<code> and ~<code>
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","make","drop Held","make","drop Held"]);
}

#[test]
fn cmtt_generation_loops_emit_each_resource_scope_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        struct Counter {remaining:.u8'builtin}
        fn make()->Held {Held {}}
        pure comptime fn repeat(count:.u8'builtin)->< |- .void'builtin> {
            let state=Counter {remaining:count};
            var code='(do {});
            while true {
                if state.remaining == 0 {return code;}
                const previous=code;
                code='(do {~<previous>;while true {let held=make();break;}});
                state.remaining -= 1;
            }
        }
        fn entry() {~<repeat(3)>;}
        fn zero() {~<repeat(0)>;}");
    for (name,expected) in [
        ("entry",vec!["entry","make","drop Held","make","drop Held","make","drop Held"]),
        ("zero",vec!["zero"]),
    ] {
        let executable=source_executable(&package,name).unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn cmtt_generators_only_emit_runtime_resources_and_preserve_affine_inputs() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        pure comptime fn generate()->< |- bool> {
            var code='(do {let held=make();true});
            const original=code;
            code='(not ~<original>);
            code='(not ~<code>);
            code
        }
        fn entry()->bool {const code=generate();~<code> and ~<code>}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","make","drop Held","make","drop Held"]);
    assert!(!package.functions.iter().any(|function|function.name=="generate"));

    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn consume(held:Held)->bool {true}
        pure comptime fn keep(code:<h:Held |- bool>)-><h:Held |- bool> {code}
        fn entry()->bool {
            let held=Held {};
            const original:<held:Held |- bool> = '(consume(held));
            const code=keep(original);
            ~<code>;~<code>
        }");
    let failure=source_executable(&package,"entry").err().expect("code transport must not duplicate affine resources");
    assert!(failure.contains("available") || failure.contains("move"),"{failure}");
}

#[test]
fn cmtt_splices_preserve_affine_consumption_and_aliasing() {
    for body in [
        "let held=Held {};const code='(consume(held));~<code>;~<code>",
        "let held=Held {};const code='(consume(held));consume(held);~<code>",
        "let first=Held {};let second=Held {};const code='[a=first,b=second](do {consume(first);consume(second)});~[a=first,b=first]<code>",
        "let first=Held {};let second=Held {};const code='[a=first,b=second](|| do {consume(first);consume(second)});let callback=~[a=first,b=first]<code>;callback()",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}}
            fn consume(value:Held)->bool {{true}}
            fn entry()->bool {{{body}}}"));
        let failure=source_executable(&package,"entry").err().expect("duplicated affine use must be rejected");
        assert!(failure.contains("available") || failure.contains("move"),"{body}: {failure}");
    }
}

#[test]
fn cmtt_quote_does_not_consume_a_resource_until_connection() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn consume(value:Held)->bool {true}
        fn entry()->bool {let held=Held {};const unused='(consume(held));consume(held)}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","consume","drop Held"]);
}

#[test]
fn enum_tag_name_cleans_payload_once_and_keeps_named_owner_usable() {
    use comptime_interpretor::{Interpreter,Value};
    for body in [
        "make()'tag_name",
        "do {let event=make();let name=event'tag_name;event match {.held(_) => name,_ => \"wrong\",}}",
        "do {let event=make();true match {_ if event'tag_name == \"held\" => event'tag_name,_ => \"wrong\",}}",
        "do {let box=Box {event:make()};box.event'tag_name}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            typealias str=.str'builtin;
            struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}}
            enum Event {{held(Held),empty,}}
            struct Box {{event:Event}}
            fn make()->Event {{Event.held(Held {{}})}}
            fn entry()->str {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::String("held".into()));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","make","drop Held"]);
    }
}

#[test]
fn enum_tag_name_rejects_moved_owners() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias str=.str'builtin;
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum Event {held(Held),empty,}
        fn entry()->str {let event=Event.held(Held {});let moved=event;event'tag_name}");
    let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
    assert!(error.contains("available"),"{error}");
}

#[test]
fn symbol_tag_name_outlives_the_owner_and_is_readable_during_drop() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Symbol=.Symbol'builtin;typealias str=.str'builtin;
        struct Packet {tag:Symbol}
        impl Retire for Packet {fn drop(*self) {assumes (self.tag.*)'tag_name == \"ready\";}}
        fn entry()->str {let packet=Packet {tag:.ready};packet.tag'tag_name}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::String("ready".into()));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Packet"]);
}

#[test]
fn symbol_values_copy_through_patterns_and_borrowed_fields() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Symbol=.Symbol'builtin;
        struct Packet {tag:Symbol} impl Retire for Packet {fn drop(*self) {assumes self.tag.* == .ready;}}
        fn entry()->bool {let packet=Packet {tag:.ready};let {tag}=packet;tag==.ready}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Packet"]);
}

#[test]
fn candidate_blocks_keep_scalar_locals_private_and_evaluate_in_order() {
    use comptime_interpretor::{Interpreter, Value};
    for expression in [
        "do {let copy=first.flag;check(copy);let inverse=not copy;not inverse}",
        "do {var copy=not first.flag;copy=not copy;check(copy)}",
        "do {let copy=first.flag;if copy {let inner=check(copy);inner} else {check(copy)}}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{flag:bool}} impl Retire for Held {{fn drop(*self) {{}}}}
            fn check(flag:bool)->bool {{flag}}
            fn entry(flag:bool)->bool {{
                (Held {{flag:flag}},Held {{flag:true}}) match {{
                    ((first,second) if {expression}) or
                    ((second,first) if {expression}) => first.flag,
                    _=>false,
                }}
            }}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error| panic!("{expression}: {error}"));
        for flag in [false,true] {
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|n| n.as_str()=="check").count(),if flag {1} else {2},"{names:?}");
            assert_eq!(names.iter().filter(|n| n.as_str()=="drop Held").count(),2,"{names:?}");
        }
    }
}

#[test]
fn candidate_blocks_reject_external_mutation_and_untracked_resources() {
    for body in [
        "outside=not outside;true",
        "let scratch=Held {};true",
        "let pointer=outside.ref;true",
        "consume(item);true",
        "return true;",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}}
            fn consume(item:Held)->bool {{true}}
            fn entry(flag:bool)->bool {{
                var outside=flag;
                Held {{}} match {{item if do {{{body}}}=>true,_=>false}}
            }}"));
        let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
        assert!(error.contains("failure-path"),"{body}: {error}");
    }
}

#[test]
fn candidate_blocks_feed_sequential_patterns_without_losing_the_owner() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flag:bool} impl Retire for Held {fn drop(*self) {let flag=self.flag.*;}}
        fn check(flag:bool)->bool {flag}
        fn entry(flag:bool)->bool {
            let item and do {let copy=item.flag;let result=check(copy);result} is true
                = Held {flag:flag} else {return false;};
            item.flag
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","check","drop Held"]);
    }
}

#[test]
fn scalar_then_patterns_chain_computed_bindings_without_transferring_subject_fields() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flag:bool} impl Retire for Held {fn drop(*self) {let flag=self.flag.*;}}
        fn flip(flag:bool)->bool {not flag}
        fn entry(flag:bool)->bool {
            let item and flip(item.flag) is second and flip(second) is third = Held {flag:flag};
            third and not second
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","flip","flip","drop Held"]);
    }
}

#[test]
fn scalar_then_patterns_retry_with_candidate_local_computed_values_and_orders() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flag:bool} impl Retire for Held {fn drop(*self) {}}
        fn check(flag:bool)->bool {flag}
        fn entry(flag:bool)->bool {
            (Held {flag:flag},Held {flag:true}) match {
                ((first,second) and check(first.flag) is (true as ready)) or
                ((second,first) and check(first.flag) is (true as ready)) => ready,
                _=>false,
            }
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|n| n.as_str()=="check").count(),if flag {1} else {2},"{names:?}");
        assert_eq!(names.iter().filter(|n| n.as_str()=="drop Held").count(),2,"{names:?}");
    }
}

#[test]
fn scalar_then_patterns_skip_computation_when_first_pattern_fails() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn check()->bool {true}
        fn entry(flag:bool)->bool {
            (flag,Held {}) match {(true,item) and check() is ready=>ready,_=>false}
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names,if flag {vec!["entry","check","drop Held"]} else {vec!["entry","drop Held"]});
    }
}

#[test]
fn scalar_then_patterns_do_not_admit_untracked_computed_owners_or_consumption() {
    for (definition,value) in [
        ("fn make()->Held {Held {}}","make()"),
        ("fn consume(item:Held)->bool {false}","consume(item)"),
        ("fn reject(flag:bool)->bool {assumes flag;flag}","reject(flag)"),
        ("","flag.ref"),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}}
            {definition}
            fn entry(flag:bool) {{let item and {value} is derived = Held {{}};}}
        "));
        let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
        assert!(error.contains("failure-path"),"{error}");
    }
}

#[test]
fn scalar_then_patterns_let_else_failure_preserves_subject_cleanup() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flag:bool} impl Retire for Held {fn drop(*self) {let flag=self.flag.*;}}
        fn entry(flag:bool)->bool {
            let item and item.flag is true = Held {flag:flag} else {return false;};
            item.flag
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held"]);
    }
}

#[test]
fn nested_observation_guards_keep_candidate_transfers_and_remapped_reads() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flag:bool} impl Retire for Held {fn drop(*self) {}}
        fn check(flag:bool)->bool {flag}
        fn entry(flag:bool)->bool {
            (Held {flag:flag},Held {flag:true}) match {
                ((first,second) if check(first.flag)) or
                ((second,first) if check(first.flag)) => first.flag,
                _=>false,
            }
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|n| n.as_str()=="check").count(),if flag {1} else {2},"{names:?}");
        assert_eq!(names.iter().filter(|n| n.as_str()=="drop Held").count(),2,"{names:?}");
    }
}

#[test]
fn nested_observation_guards_cannot_consume_tentative_owners() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn consume(item:Held)->bool {false}
        fn entry() {(Held {},Held {}) match {
            ((first,second) if consume(first)) or (second,first)=>(),
        };}");
    let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
    assert!(error.contains("guard") && error.contains("failure-path"),"{error}");
}

#[test]
fn nested_observation_guards_in_fields_and_whole_bindings_preserve_owners() {
    use comptime_interpretor::{Interpreter, Value};
    for pattern in [
        "((first if first.flag),second) or (second,(first if first.flag))",
        "(({flag:(ready if ready)} as first),second) or (second,({flag:(ready if ready)} as first))",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{flag:bool}} impl Retire for Held {{fn drop(*self) {{let flag=self.flag.*;}}}}
            fn entry(flag:bool)->bool {{
                let {pattern}=(Held {{flag:false}},Held {{flag:flag}}) else {{return false;}};
                first.flag
            }}"));
        let executable=source_executable(&package,"entry").unwrap();
        for flag in [false,true] {
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
            assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held","drop Held"]);
        }
    }
}

#[test]
fn scalar_call_pattern_guards_commit_once_or_preserve_the_subject() {
    use comptime_interpretor::{Interpreter, Value};
    for guard in ["accept(flag)", "accept(accept(flag))", "flag and accept(flag)"] {
        for flag in [false, true] {
            let package = compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}}
                fn accept(flag:bool)->bool {{flag}}
                fn entry(flag:bool) {{
                    (Held {{}}, Held {{}}) match {{
                        (first,second) if {guard} => (),
                        (first,second) => (),
                    }};
                }}"));
            let executable = source_executable(&package, "entry").unwrap();
            let mut vm = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry, vec![Value::Bool(flag)]).unwrap(), Value::Unit);
            let names = executed_names(&package, &executable, vm.events());
            let count = if guard == "accept(accept(flag))" {2}
                else if guard == "flag and accept(flag)" && !flag {0} else {1};
            assert_eq!(names.iter().filter(|name| name.as_str() == "accept").count(), count, "{names:?}");
            assert_eq!(names.iter().filter(|name| name.as_str() == "drop Held").count(), 2, "{names:?}");
        }
    }
}

#[test]
fn scalar_call_pattern_guards_support_let_else_and_callee_resources() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} struct Scratch {}
        impl Retire for Held {fn drop(*self) {}}
        impl Retire for Scratch {fn drop(*self) {}}
        fn accept(flag:bool)->bool {let scratch=Scratch {};flag}
        fn entry(flag:bool) {let item if accept(flag)=Held {} else {return;};}");
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","accept","drop Scratch","drop Held"]);
    }
}

#[test]
fn scalar_call_pattern_guards_observe_tentative_fields_across_candidates() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flag:bool} impl Retire for Held {fn drop(*self) {}}
        enum Choice {one(Held,Held),two(bool,Held,Held)}
        fn accept(flag:bool)->bool {flag}
        fn entry(which:bool,flag:bool)->bool {
            let value=if which {Choice.one(Held {flag:flag},Held {flag:false})}
                else {Choice.two(false,Held {flag:false},Held {flag:flag})};
            value match {
                .one(first,second) or .two(_,second,first) if accept(first.flag)=>true,
                _=>false,
            }
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for which in [false,true] { for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(which),Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","accept","drop Held","drop Held"]);
    }}
}

#[test]
fn scalar_call_pattern_guards_reject_transfer_and_contract_failure() {
    for (definition, call) in [
        ("fn accept(item:Held)->bool {true}", "accept(item)"),
        ("fn accept(flag:bool)->bool {assumes flag;flag}", "accept(flag)"),
        ("fn accept(flag:*bool)->bool {flag.*}", "accept(flag.ref)"),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}}
            {definition}
            fn entry(flag:bool) {{Held {{}} match {{item if {call}=>(),_=>()}};}}"));
        let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
        assert!(error.contains("guard") && error.contains("failure-path"),"{error}");
    }
}

#[test]
fn candidate_order_patterns_keep_copy_choices_inside_whole_bindings() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flags:(bool,bool)} impl Retire for Held {fn drop(*self) {}}
        fn entry(flag:bool,inner:bool)->bool {
            let value=(flag,Held {flags:(inner,not inner)},Held {flags:(inner,not inner)});
            value match {
                (true,({flags:((true as a,false as b) or (false as b,true as a))} as first),second) or
                (false,second,({flags:((true as a,false as b) or (false as b,true as a))} as first))=>a and not b,
                _=>false,
            }
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {for inner in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag),Value::Bool(inner)]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held","drop Held"]);
    }}
}

#[test]
fn candidate_order_patterns_calls_restore_caller_receipts_and_validate_metadata() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        fn left() {} fn right() {}
        fn nested(flag:bool) {let ((true,first,second) or (false,second,first))=(flag,Held {tag:true},Held {tag:false});}
        fn entry() {let ((true,first,second) or (false,second,first))=(false,Held {tag:true},Held {tag:false});
            nested(true);nested(false);
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","nested","drop Held","right","drop Held","left",
        "nested","drop Held","right","drop Held","left","drop Held","right","drop Held","left"]);
    for defect in 0..4 {
        let mut schedule=executable.schedule.clone();
        let function=schedule.functions.iter_mut().find(|function|!function.pattern_orders.is_empty()).unwrap();
        match defect {
            0=>function.pattern_orders[0].alternatives[1]=function.pattern_orders[0].alternatives[0].clone(),
            1=>{function.pattern_orders[0].alternatives[1].pop();},
            2=>{let binding=function.pattern_orders[0].alternatives[0][0];function.pattern_orders[0].alternatives[0].push(binding);},
            3=>function.pattern_orders.push(function.pattern_orders[0].clone()),
            _=>unreachable!(),
        }
        assert!(Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&schedule).is_err(),"defect {defect}");
    }
}

#[test]
fn candidate_order_patterns_cross_variants_let_guard_and_early_return() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        enum E {one(Held,Held),two(bool,Held,Held)}
        fn left() {} fn right() {}
        fn work(value:E,accept:bool) {
            let (.one(first,second) or .two(_,second,first)) if accept=value else {return;};
            return;
        }
        fn entry(flag:bool,accept:bool) {work(if flag {E.one(Held {tag:true},Held {tag:false})}
            else {E.two(false,Held {tag:true},Held {tag:false})},accept);}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {for accept in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag),Value::Bool(accept)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","work","drop Held","right","drop Held","left"]);
    }}
}

#[test]
fn candidate_order_patterns_consumed_bindings_and_replacements_keep_original_rank() {
    use comptime_interpretor::{Interpreter,Value};
    for body in ["take(first);", "first = Held {tag:false};"] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{tag:bool}} impl Retire for Held {{fn drop(*self) {{if self.tag.* {{left();}} else {{right();}}}}}}
            fn left() {{}} fn right() {{}} fn take(value:Held) {{}}
            fn entry(flag:bool) {{var ((true,first,second) or (false,second,first))=(flag,Held {{tag:true}},Held {{tag:false}});
                {body}
            }}"));
        let executable=source_executable(&package,"entry").unwrap();
        for flag in [false,true] {
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
            let expected=if body.starts_with("take") {
                if flag {vec!["entry","take","drop Held","left","drop Held","right"]}
                else {vec!["entry","take","drop Held","right","drop Held","left"]}
            } else if flag {vec!["entry","drop Held","left","drop Held","right","drop Held","right"]}
                else {vec!["entry","drop Held","right","drop Held","right","drop Held","left"]};
            assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}, {flag}");
        }
    }
}

#[test]
fn candidate_order_patterns_check_dependencies_for_every_cleanup_order() {
    for (pattern, accepted) in [("(_,first,second)",true),("((true,first,second) or (false,second,first))",false)] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{flag:bool,target:*bool}}
            impl Retire for Held {{fn drop(*self) {{let observed=self.target.*.*;}}}}
            fn entry(flag:bool) {{let external=true;
                let {pattern}=(flag,Held {{flag:true,target:external.ref}},Held {{flag:false,target:external.ref}});
                second.target=first.flag.ref;
            }}"));
        let result=source_executable(&package,"entry");
        assert_eq!(result.is_ok(),accepted,"{pattern}: {result:?}");
    }
}

#[test]
fn candidate_order_patterns_loop_replaces_the_previous_successful_order() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        fn left() {} fn right() {}
        fn entry() {var flag=true;while true {
            let ((true,first,second) or (false,second,first))=(flag,Held {tag:true},Held {tag:false});
            if not flag {break;}flag=false;
        }}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held","right","drop Held","left","drop Held","right","drop Held","left"]);
}

#[test]
fn candidate_order_patterns_retire_in_successful_binding_order() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        struct Before {} struct After {}
        impl Retire for Before {fn drop(*self) {}} impl Retire for After {fn drop(*self) {}}
        fn left() {} fn right() {}
        fn entry(flag:bool) {let before=Before {};let value=(flag,Held {tag:true},Held {tag:false});
            value match {(true,first,second) or (false,second,first)=>{let after=After {};}};
        }");
    let executable=source_executable(&package,"entry").unwrap();
    assert!(executable.schedule.to_string().contains("pattern-order"));
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop After","drop Held","right","drop Held","left","drop Before"]);
    }
}

#[test]
fn conditional_variant_patterns_nested_layouts_preserve_parent_drop_and_order() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        enum Inner {pair(Held,Held),swapped(bool,Held,Held)}
        enum Outer {nested(Inner),flat(Held,Held)}
        impl Retire for Outer {fn drop(*self) {parent();}}
        fn parent() {} fn left() {} fn right() {} fn take(value:Held) {}
        fn select(value:Outer) {value match {
            .nested(.pair(item,_)) or .nested(.swapped(_,_,item)) or .flat(_,item)=>take(item),
        };}
        fn entry(outer:bool,inner:bool) {
            select(if outer {Outer.nested(if inner {Inner.pair(Held {tag:true},Held {tag:false})}
                else {Inner.swapped(false,Held {tag:true},Held {tag:false})})}
                else {Outer.flat(Held {tag:true},Held {tag:false})});
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for outer in [false,true] {for inner in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(outer),Value::Bool(inner)]).unwrap(),Value::Unit);
        let (first,last)=if outer && inner {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select","take","drop Held",first,"drop Outer","parent","drop Held",last]);
    }}
}

#[test]
fn conditional_variant_patterns_join_same_tag_transfers_with_other_layouts() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        enum E {pair(bool,Held,Held),other(Held,Held)}
        fn left() {} fn right() {} fn take(value:Held) {}
        fn select(value:E) {value match {
            .pair(true,item,_) or .pair(false,_,item) or .other(item,_)=>take(item),
        };}
        fn entry(tag:bool,flag:bool) {select(if tag {E.pair(flag,Held {tag:true},Held {tag:false})}
            else {E.other(Held {tag:true},Held {tag:false})});}");
    let executable=source_executable(&package,"entry").unwrap();
    assert!(executable.schedule.to_string().contains("cleanup-if-constructed"));
    for tag in [false,true] {for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag),Value::Bool(flag)]).unwrap(),Value::Unit);
        let (first,last)=if !tag || flag {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select","take","drop Held",first,"drop Held",last]);
    }}
}

#[test]
fn conditional_variant_patterns_let_else_and_guard_failure_keep_owners() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        enum E {left(Held,Held),right(bool,Held,Held)}
        fn left() {} fn right() {} fn take(value:Held) {}
        fn select(value:E,accept:bool) {
            let (.left(item,_) or .right(_,_,item)) if accept=value else {return;};
            take(item);
        }
        fn entry(tag:bool,accept:bool) {select(if tag {E.left(Held {tag:true},Held {tag:false})}
            else {E.right(false,Held {tag:true},Held {tag:false})},accept);}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {for accept in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag),Value::Bool(accept)]).unwrap(),Value::Unit);
        let (selected,residual)=if tag {("left","right")} else {("right","left")};
        let expected=if accept {vec!["entry","select","drop Held",residual,"take","drop Held",selected]}
            else {vec!["entry","select","drop Held","right","drop Held","left"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }}
}

#[test]
fn conditional_variant_patterns_return_cleans_binding_before_residual() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        enum E {left(Held,Held),right(bool,Held,Held)}
        fn left() {} fn right() {}
        fn select(value:E,accept:bool) {value match {
            .left(item,_) or .right(_,_,item) if accept=>{return;},
            _=>(),
        };}
        fn entry(tag:bool,accept:bool) {select(if tag {E.left(Held {tag:true},Held {tag:false})}
            else {E.right(false,Held {tag:true},Held {tag:false})},accept);}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {for accept in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag),Value::Bool(accept)]).unwrap(),Value::Unit);
        let (first,last)=if accept && tag {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select","drop Held",first,"drop Held",last]);
    }}
}

#[test]
fn conditional_affine_patterns_nested_named_fields_keep_exact_residuals() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        struct Pair {left:Held,right:Held} struct Packet {tag:bool,pair:Pair}
        fn left() {} fn right() {} fn take(value:Held) {}
        fn entry(flag:bool) {let value=Packet {tag:flag,pair:Pair {left:Held {tag:true},right:Held {tag:false}}};
            value match {
                {pair:{left:item},tag:true} or {tag:false,pair:{right:item}}=>take(item),
            };}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        let (first,last)=if flag {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Held",first,"drop Held",last]);
    }
}

#[test]
fn conditional_affine_patterns_failed_guard_retries_with_intact_subject() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        fn left() {} fn right() {} fn take(value:Held) {}
        fn entry(tag:bool,accept:bool) {let value=(tag,Held {tag:true},Held {tag:false});
            value match {
                (true,item,_) or (false,_,item) if accept=>take(item),
                (_,item,_)=>take(item),
            };}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {for accept in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag),Value::Bool(accept)]).unwrap(),Value::Unit);
        let (first,last)=if !accept || tag {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Held",first,"drop Held",last]);
    }}
}

#[test]
fn conditional_affine_patterns_let_else_cleans_success_and_failure() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        fn left() {} fn right() {} fn take(value:Held) {}
        fn entry(tag:bool,accept:bool) {
            let ((true,item,_) or (false,_,item)) if accept=(tag,Held {tag:true},Held {tag:false}) else {return;};
            take(item);
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {for accept in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag),Value::Bool(accept)]).unwrap(),Value::Unit);
        let (selected,residual)=if tag {("left","right")} else {("right","left")};
        let expected=if accept {vec!["entry","drop Held",residual,"take","drop Held",selected]}
            else {vec!["entry","drop Held","right","drop Held","left"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }}
}

#[test]
fn conditional_affine_patterns_nested_or_and_early_return_keep_order() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        fn left() {} fn right() {}
        fn entry(tag:bool) {let value=((tag,Held {tag:true},Held {tag:false}),false);
            value match {((true,item,_) or (false,_,item),_)=>{return;}};}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag)]).unwrap(),Value::Unit);
        let (selected,residual)=if tag {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held",selected,"drop Held",residual]);
    }
}

#[test]
fn conditional_affine_patterns_select_variant_specific_cleanup() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        enum E {left(Held,Held),right(bool,Held,Held)}
        fn left() {} fn right() {} fn take(value:Held) {}
        fn select(value:E) {value match {.left(item,_) or .right(_,_,item)=>take(item)};}
        fn entry(flag:bool) {select(if flag {E.left(Held {tag:true},Held {tag:false})} else {E.right(false,Held {tag:true},Held {tag:false})});}");
    let executable=source_executable(&package,"entry").unwrap();
    assert!(executable.schedule.to_string().contains("cleanup-select"));
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        let (first,last)=if flag {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select","take","drop Held",first,"drop Held",last]);
    }
}

#[test]
fn ordinary_or_copy_patterns_select_different_variant_fields() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {left(bool,Held),right(Held,bool),}
        fn select(value:E)->bool {value match {.left(flag,_) or .right(_,flag)=>flag}}
        fn entry()->bool {select(E.left(true,Held {})) and not select(E.right(Held {},false))}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select","drop Held","select","drop Held"]);
}

#[test]
fn ordinary_or_copy_patterns_keep_all_parent_fields_for_drop() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Packet {tag:bool,left:bool,right:bool}
        impl Retire for Packet {fn drop(*self) {assumes self.left.* and not self.right.*;}}
        fn select(value:Packet)->bool {value match {
            {tag:true,left:flag} or {tag:false,right:flag}=>flag,
        }}
        fn entry(tag:bool)->bool {select(Packet {tag:tag,left:true,right:false})}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag)]).unwrap(),Value::Bool(tag));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select","drop Packet"]);
    }
}

#[test]
fn ordinary_or_copy_patterns_failed_candidates_leave_subject_cleanup_intact() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {left(Held,bool),right(bool,Held),}
        fn accepted() {}
        fn entry(tag:bool)->bool {
            let (.left(_,true as result) or .right(false as result,_))=E.right(tag,Held {}) else {return true;};
            accepted();not result
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag)]).unwrap(),Value::Bool(true));
        let expected=if tag {vec!["entry","drop Held"]} else {vec!["entry","drop Held","accepted"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn ordinary_copy_pattern_keeps_field_available_to_parent_drop() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Packet {flag:bool}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.*;}}
        fn entry()->bool {let {flag}=Packet {flag:true};flag}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Packet"]);
}

#[test]
fn ordinary_copy_pattern_mixes_copy_fields_with_affine_transfers() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        struct Packet {flag:bool,held:Held}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.*;}}
        fn take(value:Held) {}
        fn entry()->bool {let {flag,held}=Packet {flag:true,held:Held {}};take(held);flag}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Packet","take","drop Held"]);
}

#[test]
fn ordinary_copy_pattern_retains_nested_fields_for_each_parent_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Inner {flag:bool} struct Outer {inner:Inner}
        impl Retire for Inner {fn drop(*self) {assumes self.flag.*;}}
        impl Retire for Outer {fn drop(*self) {assumes self.inner.flag.*;}}
        fn entry()->bool {let {inner:{flag}}=Outer {inner:Inner {flag:true}};flag}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Outer","drop Inner"]);
}

#[test]
fn ordinary_copy_pattern_uses_declared_copy_for_nominal_fields() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Flag {value:bool} impl Duplicate for Flag {}
        struct Packet {flag:Flag}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.value.*;}}
        fn entry()->bool {let {flag}=Packet {flag:Flag {value:true}};flag.value}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Packet"]);
    let affine=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Flag {value:bool} struct Packet {flag:Flag}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.value.*;}}
        fn entry()->bool {let {flag}=Packet {flag:Flag {value:true}};flag.value}");
    let error=source_executable(&affine,"entry").unwrap_err();
    assert!(error.contains("unavailable"),"{error}");
}

#[test]
fn ordinary_copy_pattern_guard_and_or_keep_the_residual_parent_readable() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Packet {tag:bool,flag:bool}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.*;}}
        fn accepted() {}
        fn select(value:Packet,allow:bool)->bool {value match {
            {tag:true,flag:ready} or {tag:false,flag:ready} if allow=>{accepted();ready},
            _=>false,
        }}
        fn entry(tag:bool,allow:bool)->bool {select(Packet {tag:tag,flag:true},allow)}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {for allow in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag),Value::Bool(allow)]).unwrap(),Value::Bool(allow));
        let expected=if allow {vec!["entry","select","accepted","drop Packet"]} else {vec!["entry","select","drop Packet"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }}
}

#[test]
fn conditional_copy_patterns_select_fields_without_splitting_the_owner() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {left(bool,Held),right {held:Held,value:bool},}
        fn select(value:E)->bool {value match {
            (.left(selected,_) or .right {value:selected}) as whole=>selected,
        }}
        fn entry()->bool {select(E.left(true,Held {})) and not select(E.right {held:Held {},value:false})}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select","drop Held","select","drop Held"]);
}

#[test]
fn conditional_copy_patterns_nested_choices_and_swapped_binding_order() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn choose(tag:bool)->bool {
            let ((true,a,b,_) or (false,b,a,_)) as whole=(tag,true,false,Held {});
            if tag {a and not b} else {b and not a}
        }
        fn entry()->bool {choose(true) and choose(false)}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose","drop Held","choose","drop Held"]);
}

#[test]
fn conditional_copy_patterns_nested_remaps_and_guard_rollback() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {left(bool,Held),right(Held,bool),}
        fn accepted() {}
        fn keep(value:E,allow:bool)->E {value match {
            ((.left(flag,_) or .right(_,flag)) or .left(flag,_)) as whole if flag and allow=>{accepted();whole},
            whole=>whole,
        }}
        fn entry(allow:bool) {let value=keep(E.right(Held {},true),allow);}");
    let executable=source_executable(&package,"entry").unwrap();
    for allow in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(allow)]).unwrap(),Value::Unit);
        let expected=if allow {vec!["entry","keep","accepted","drop Held"]} else {vec!["entry","keep","drop Held"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn conditional_copy_patterns_join_selected_callback_evidence() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Action=fn(fn(bool)->bool,bool)->bool;impl Duplicate for Action {}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {left(Action,Held),right(Held,Action),}
        fn negate(value:bool)->bool {not value}
        fn apply(action:fn(bool)->bool,value:bool)->bool {action(value)}
        fn relay(action:fn(bool)->bool,value:bool)->bool {action(value)}
        fn select(value:E)->bool {value match {
            (.left(action,_) or .right(_,action)) as whole=>action(negate,false),
        }}
        fn entry(flag:bool)->bool {select(if flag {E.left(apply,Held {})} else {E.right(Held {},relay)})}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","select",if flag {"apply"} else {"relay"},"negate","drop Held"]);
    }
}

#[test]
fn conditional_copy_patterns_reject_affine_extraction_in_any_candidate() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {left(Held),right(Held),}
        fn entry(value:E) {value match {(.left(item) or .right(item)) as whole=>()};}");
    let failure=source_executable(&package,"entry").unwrap_err();
    assert!(failure.contains("requires Copy"),"{failure}");
}

#[test]
fn overlapping_copy_patterns_keep_affine_parent_intact() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        struct Packet {flag:bool,held:Held}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.*;}}
        fn keep(value:Packet)->Packet {let {flag:ready} as whole=value;assumes ready;whole}
        fn entry() {let packet=keep(Packet {flag:true,held:Held {}});}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","keep","drop Packet","drop Held"]);
}

#[test]
fn overlapping_copy_patterns_nominal_copy_is_independent_of_whole() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Flag {value:bool} impl Duplicate for Flag {}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        struct Packet {flag:Flag,held:Held}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.value.*;}}
        fn entry()->bool {
            let {flag:copied} as whole=Packet {flag:Flag {value:true},held:Held {}};
            var changed=copied;changed.value=false;
            whole.flag.value and not changed.value
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Packet","drop Held"]);
}

#[test]
fn overlapping_copy_patterns_nested_bindings_copy_each_additional_owner() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        typealias Pair=(bool,bool);impl Duplicate for Pair {}
        fn entry()->bool {
            let ((left,right) as pair,_) as whole=((true,false),Held {});
            let (copy_left,copy_right)=pair;
            let ((original_left,original_right),_)=whole;
            left and not right and copy_left and not copy_right and original_left and not original_right
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held"]);
}

#[test]
fn overlapping_copy_patterns_guard_retry_preserves_affine_enum() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {one(bool,Held),empty,}
        fn accepted() {}
        fn keep(value:E,accept:bool)->E {value match {
            (.one(flag,_) as whole) if flag and accept => {accepted();whole},
            whole=>whole,
        }}
        fn entry(accept:bool) {let value=keep(E.one(true,Held {}),accept);}");
    let executable=source_executable(&package,"entry").unwrap();
    for accept in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(accept)]).unwrap(),Value::Unit);
        let expected=if accept {vec!["entry","keep","accepted","drop Held"]} else {vec!["entry","keep","drop Held"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn overlapping_copy_patterns_work_with_canonical_or_bindings() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {one(bool,bool,Held),empty,}
        fn entry(tag:bool)->bool {let value=E.one(tag,true,Held {});
            value match {
                (.one(true,flag,_) as whole) or (.one(false,flag,_) as whole)=>flag,
                .empty=>false,
            }
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag)]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held"]);
    }
}

#[test]
fn overlapping_copy_patterns_reject_unproved_and_invalid_copy_claims() {
    for extra in ["", "impl Duplicate for Held {}"] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}} {extra}
            struct Packet {{held:Held}}
            fn entry() {{let {{held:item}} as whole=Packet {{held:Held {{}}}};}}"));
        assert!(source_executable(&package,"entry").is_err(),"accepted {extra}");
    }
}

#[test]
fn overlapping_copy_patterns_preserve_both_callback_evidence_paths() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        typealias Action=fn(fn(bool)->bool,bool)->bool;impl Duplicate for Action {}
        struct Job {action:fn(fn(bool)->bool,bool)->bool,held:Held}
        fn negate(value:bool)->bool {not value}
        fn apply(action:fn(bool)->bool,value:bool)->bool {action(value)}
        fn entry()->bool {
            let {action:copied} as whole=Job {action:apply,held:Held {}};
            copied(negate,false) and whole.action(negate,false)
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","negate","apply","negate","drop Held"]);
}

#[test]
fn bound_or_patterns_move_selected_payload_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {one(bool,Held),empty,}
        fn take(value:Held) {}
        fn entry(flag:bool) {let value=E.one(flag,Held {});
            value match {.one(true,item) or .one(false,item)=>take(item),.empty=>()};}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Held"]);
    }
}

#[test]
fn bound_or_patterns_roll_back_early_bindings_before_retry() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {one(Held,bool),empty,}
        fn take(value:Held) {}
        fn entry(flag:bool) {let value=E.one(Held {},flag);
            value match {.one(item,true) or .one(item,false)=>{take(item);return;},.empty=>()};}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Held"]);
    }
}

#[test]
fn bound_or_patterns_guard_failure_keeps_original_payloads() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct First {} struct Second {}
        impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}}
        enum E {pair(bool,First,Second),empty,}
        fn take(first:First,second:Second) {}
        fn entry(tag:bool,flag:bool) {let value=E.pair(tag,First {},Second {});
            value match {
                .pair(true,first,second) or .pair(false,first,second) if flag=>take(first,second),
                _=>(),
            };}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag),Value::Bool(flag)]).unwrap(),Value::Unit);
        let expected=if flag {vec!["entry","take","drop Second","drop First"]} else {vec!["entry","drop Second","drop First"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }}
}

#[test]
fn bound_or_patterns_named_fields_and_nested_alternatives_share_bindings() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct First {} struct Second {}
        impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}}
        struct Pair {tag:bool,first:First,second:Second}
        fn take(first:First,second:Second) {}
        fn entry(tag:bool) {let value=Pair {tag:tag,first:First {},second:Second {}};
            let (({tag:true,first:a,second:b} or {second:b,tag:false,first:a})
                or {first:a,second:b,tag:true})=value else {return;};take(a,b);}");
    let executable=source_executable(&package,"entry").unwrap();
    for tag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(tag)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Second","drop First"]);
    }
}

#[test]
fn conditional_affine_patterns_transfer_selected_tuple_field_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {tag:bool} impl Retire for Held {fn drop(*self) {if self.tag.* {left();} else {right();}}}
        fn left() {} fn right() {}
        fn take(value:Held) {}
        fn entry(flag:bool) {let value=(flag,Held {tag:true},Held {tag:false});
            value match {(true,item,_) or (false,_,item)=>take(item)};}");
    let executable=source_executable(&package,"entry").unwrap();
    assert!(executable.schedule.to_string().contains("pattern-transfers"));
    assert!(executable.schedule.to_string().contains("if-constructed"));
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        let (first,last)=if flag {("left","right")} else {("right","left")};
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Held",first,"drop Held",last]);
    }
}

#[test]
fn bound_or_patterns_can_transfer_whole_values_across_distinct_variants() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {one(Held),two {item:Held},empty,}
        fn keep(value:E)->E {value match {
            (.one(_) as whole) or (.two {} as whole)=>whole,
            .empty=>E.empty,
        }}
        fn entry() {let first=keep(E.one(Held {}));let second=keep(E.two {item:Held {}});}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","keep","keep","drop Held","drop Held"]);
}

#[test]
fn bound_or_patterns_failed_let_else_keeps_subject_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} struct Caller {}
        impl Retire for Held {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}}
        enum E {one(Held,bool,bool),empty,}
        fn take(value:Held) {}
        fn entry(a:bool,b:bool) {let caller=Caller {};
            let (.one(item,true,_) or .one(item,_,true))=E.one(Held {},a,b) else {return;};
            take(item);
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for a in [false,true] {for b in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(a),Value::Bool(b)]).unwrap(),Value::Unit);
        let expected=if a || b {vec!["entry","take","drop Held","drop Caller"]} else {vec!["entry","drop Held","drop Caller"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }}
}

#[test]
fn bound_or_patterns_preserve_higher_order_callback_evidence() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        enum E {one(bool,fn(fn(bool)->bool,bool)->bool),empty,}
        fn negate(value:bool)->bool {not value}
        fn apply(action:fn(bool)->bool,value:bool)->bool {action(value)}
        fn relay(value:E)->bool {value match {
            .one(true,action) or .one(false,action)=>action(negate,false),
            .empty=>false,
        }}
        fn entry(flag:bool)->bool {relay(E.one(flag,apply))}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","relay","apply","negate"]);
    }
}

#[test]
fn observation_patterns_bind_the_whole_affine_value_without_duplication() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {group.{one(Held),two {value:Held}},empty,}
        fn keep(value:E)->E {value match {(.group) as whole=>whole,.empty=>E.empty}}
        fn entry() {let a=keep(E.group.one(Held {}));let b=keep(E.group.two {value:Held {}});}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","keep","keep","drop Held","drop Held"]);
}

#[test]
fn observation_patterns_retain_nested_payloads_across_alternatives() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {one(bool,Held),two {flag:bool,value:Held},empty,}
        fn select(value:E)->bool {value match {
            .one(true,_) or .two {flag:true} => true,
            _ => false,
        }}
        fn entry()->bool {select(E.one(true,Held {}))
            and select(E.two {flag:true,value:Held {}})
            and not select(E.one(false,Held {}))}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","select","drop Held","select","drop Held","select","drop Held"]);
}

#[test]
fn observation_patterns_commit_whole_binding_only_after_guard_success() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {one(bool,Held),two {flag:bool,value:Held},empty,}
        fn accepted() {}
        fn keep(value:E,flag:bool)->E {value match {
            (.one(true,_) or .two {flag:true}) as whole if flag => {accepted();return whole;},
            whole => whole,
        }}
        fn entry(flag:bool) {let result=keep(E.one(true,Held {}),flag);}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        let expected=if flag {vec!["entry","keep","accepted","drop Held"]} else {vec!["entry","keep","drop Held"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn observation_patterns_failed_let_else_retires_original_subject_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} struct Caller {}
        impl Retire for Held {fn drop(*self) {}}
        impl Retire for Caller {fn drop(*self) {}}
        enum E {one(bool,Held),empty,}
        fn accepted() {}
        fn entry(flag:bool) {let caller=Caller {};
            let (.one(true,_) or .empty) as whole=E.one(flag,Held {}) else {return;};
            accepted();
        }");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        let expected=if flag {vec!["entry","accepted","drop Held","drop Caller"]} else {vec!["entry","drop Held","drop Caller"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn observation_patterns_keep_parent_destructor_access_to_unmoved_fields() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        struct Packet {flag:bool,held:Held}
        impl Retire for Packet {fn drop(*self) {assumes self.flag.*;}}
        fn keep(value:Packet)->Packet {value match {{flag:true} as whole=>whole,whole=>whole}}
        fn entry() {let packet=keep(Packet {flag:true,held:Held {}});}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","keep","drop Packet","drop Held"]);
}

#[test]
fn observation_patterns_do_not_authorize_overlapping_or_repeated_moves() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Held {}
        impl Retire for Held {fn drop(*self) {}} enum E {one(Held),empty,}";
    for body in [
        "fn entry(value:E) {value match {.one(item) as whole=>(),.empty=>()};}",
        "fn consume(value:E) {} fn entry(value:E) {value match {.one(_) as whole=>{consume(whole);consume(whole);},.empty=>()};}",
    ] {
        let package=compile(&format!("{prefix}{body}"));
        assert!(source_executable(&package,"entry").is_err(),"accepted {body}");
    }
}

#[test]
fn observation_patterns_support_tuple_and_anonymous_record_owners() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Held {}
        impl Retire for Held {fn drop(*self) {}}";
    for body in [
        "fn entry() {let (true,_) as whole=(true,Held {}) else {return;};}",
        "fn build(...properties:T)->T where T {properties}
         fn entry() {let value=build {flag:true,held:Held {}};
            let {flag:true} as whole=value else {return;};}",
    ] {
        let package=compile(&format!("{prefix}{body}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        let expected=if body.contains("fn build") {vec!["entry","build","drop Held"]} else {vec!["entry","drop Held"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn whole_enum_group_matches_preserve_selected_affine_payload_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum E {group.{one(Held),two {value:Held}},empty,}
        fn consume(value:E)->bool {value match {.group=>true,.empty=>false}}
        fn entry()->bool {consume(E.group.one(Held {})) and consume(E.group.two {value:Held {}})}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","consume","drop Held","consume","drop Held"]);
}

#[test]
fn named_affine_field_is_cleaned_when_a_later_declared_field_returns() {
    use comptime_interpretor::{Interpreter,Value};
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} struct Pair {a:Held,b:Held}
        fn retired() {} impl Retire for Held {fn drop(*self) {retired();}}
        fn entry()->bool {let pair=Pair {b:do {return true;},a:Held {}};false}");
    let executable = source_executable(&package,"entry").unwrap();
    let mut vm = Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Held","retired"]);
}

use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn extended_post_do_cleans_captured_owner_exactly_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;fn take(item:Item) {} fn apply(lambda action:%{item:Item} fn(),...properties:Options) {action();} fn entry() {let item=Item {};apply {flag:true} do {take(item);};}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","take","drop Item"]);
}

#[test]
fn tactic_lambda_owned_placeholders_use_ordinary_affine_delivery() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn take(item:Item) {} fn apply(item:Item,lambda action:fn(Item)) {action(item);}";
    let package=compile(&format!("{prefix} fn entry() {{apply(Item {{}},take(_0));}}"));
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","take","drop Item"]);
    let package=compile(&format!("{prefix} fn entry() {{apply(Item {{}},do {{take(_0);take(_0);}});}}"));
    assert!(source_executable(&package,"entry").is_err());
}

#[test]
fn post_do_capture_transfers_owned_input_and_cleans_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn consume(implicit item:Item) {} fn apply(lambda action:%{item:Item} fn()) {action();} fn entry() {let item=Item {};apply() do {consume(.item=item);};}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","consume","drop Item"]);
}

#[test]
fn implicit_explicit_supply_transfers_and_cleans_owned_arguments_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn supply()->Item {Item {}} fn consume(implicit item:Item) {} fn entry() {consume(.item=supply());}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","supply","consume","drop Item"]);
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn consume(implicit item:Item) {} fn entry() {let item=Item {};consume(.item=item);consume(.item=item);}");
    assert!(source_executable(&package,"entry").is_err());
}

#[test]
fn representation_copy_preserves_pointer_dependencies() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} struct View {ptr:*bool} impl Duplicate for View {} fn entry()->bool {let flag=true;let view=View {ptr:flag.ref};let copied=view;copied.ptr.* = false;not (view.ptr.*)}");
    assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
}

#[test]
fn representation_copy_checks_every_enum_variant() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} enum Choice {none,some(bool)} impl Duplicate for Choice {} fn read(value:Choice)->bool {value match {.some(flag)=>flag,.none=>false}} fn entry()->bool {let original=Choice.some(true);let first=original;let second=original;read(first) and read(second)}");
    assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} struct Token {} enum Choice {none,some(Token)} impl Duplicate for Choice {} fn take(value:Choice) {} fn entry() {let original=Choice.none;take(original);take(original);}");
    assert!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap_err().contains("no Copy capability"));
}

#[test]
fn representation_copy_evidence_is_bounded_and_recomputed_in_each_world() {
    use elaboration::verification::copy;
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} struct Point {flag:bool} impl Duplicate for Point {}");
    let ty=nominal(&package,"Point",vec![]);
    world(&package,|world,_| {
        let protocol=|name|package.traits.iter().find(|t|t.name==name).unwrap().declaration;
        let acquire=|fuel:&mut usize|copy::acquire(&package,world,protocol("Duplicate"),protocol("Retire"),&ty,Budget::default(),fuel);
        assert!(acquire(&mut 1).unwrap_err().contains("budget"));
        let evidence=acquire(&mut 100_000).unwrap();
        assert_eq!(evidence.ty(),&ty);
        assert!(evidence.to_string().starts_with("(copy-evidence (components"));
        assert!(evidence.to_string().contains("copy-classification intrinsic"));
    });
    let mut changed=package.clone();
    changed.implementations.clear();
    world(&changed,|world,_| {
        let protocol=|name|changed.traits.iter().find(|t|t.name==name).unwrap().declaration;
        assert!(copy::acquire(&changed,world,protocol("Duplicate"),protocol("Retire"),&ty,Budget::default(),&mut 100_000).unwrap_err().contains("no Copy capability"));
    });
}

#[test]
fn representation_copy_keeps_nominal_values_independent() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} struct Point {flag:bool} impl Duplicate for Point {} fn entry()->bool {var original=Point {flag:true};let first=original;let second=original;original.flag=false;first.flag and second.flag and not original.flag}");
    assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
}

#[test]
fn representation_copy_checks_instantiated_nested_components() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} struct Cell where T {value:T} impl Duplicate for Cell<T> where T {} struct Outer {cell:Cell<bool>} impl Duplicate for Outer {} fn entry()->bool {let original=Outer {cell:Cell<bool> {value:true}};let first=original;let second=original;first.cell.value and second.cell.value}");
    assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
}

#[test]
fn representation_copy_rejects_hidden_affine_and_drop_responsibilities() {
    for (definitions, construct, reason) in [
        ("struct Inner {flag:bool} struct Outer {inner:Inner} impl Duplicate for Outer {}", "Outer {inner:Inner {flag:true}}", "no Copy capability"),
        ("struct Outer {flag:bool} impl Duplicate for Outer {} impl Retire for Outer {fn drop(*self) {}}", "Outer {flag:true}", "Drop responsibility"),
        ("struct Inner {flag:bool} impl Duplicate for Inner {} impl Retire for Inner {fn drop(*self) {}} struct Outer {inner:Inner} impl Duplicate for Outer {}", "Outer {inner:Inner {flag:true}}", "Drop responsibility"),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {definitions} fn take(value:Outer) {{}} fn entry() {{let original={construct};take(original);take(original);}}"));
        let failure=luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap_err();
        assert!(failure.contains(reason),"{failure}");
    }
}
fn compile(source: &str) -> nir::program::Package {
    compile_in(source, 0)
}
fn compile_in(source: &str, identity: u64) -> nir::program::Package {
    compile_with_optional(source, identity, None)
}
fn compile_with_optional(source: &str, identity: u64, optional: Option<&str>) -> nir::program::Package {
    let path = std::env::temp_dir().join(format!(
        "luna-drop-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let artifact = frontend::compile_in_package(
        &Options {
            input: path.clone(),
            emit: Emit::Nir,
            output: None,
            sysroot: None,
            splice_protocol: None,
            meta_list_type: None,
            index_protocol: None, arithmetic_protocols:Default::default(), tuple_type: None, optional_type: optional.map(str::to_owned),
        },
        identity,
    );
    std::fs::remove_file(path).unwrap();
    let Artifact::Nir(package) = artifact.unwrap() else {
        panic!("expected NIR")
    };
    package
}

use elaboration::{
    capabilities::World,
    implementation::{ImplementationHead, Index},
    ptc::{Budget, Context},
    verification::drop::Resolution,
};
use intrinsic::{LangItem, LangItems};
fn world<T>(package: &nir::program::Package, run: impl FnOnce(&World<'_>, &LangItems) -> T) -> T {
    let context = Context::from_package(package).unwrap();
    let mut index = Index::default();
    for i in package.implementations.iter().filter(|implementation| implementation.interface.is_some()) {
        index
            .insert(
                ImplementationHead {
                    declaration: i.declaration,
                    interface: i.interface,
                    interface_arguments: i.interface_arguments.clone(),
                    subject: i.subject.clone(),
                    type_parameters: i.type_parameters.clone(),
                    requirements: i.requirements.clone(),
                },
                Budget::default(),
            )
            .unwrap();
    }
    let protocol = package
        .traits
        .iter()
        .find(|t| t.name == "Retire")
        .unwrap()
        .declaration;
    let mut items = LangItems::new();
    items
        .bind_source_traits(package, &[(LangItem::Drop, protocol)])
        .unwrap();
    run(
        &World {
            index: &index,
            implementations: &package.implementations,
            traits: &package.traits,
            assumptions: &[],
            context: &context,
        },
        &items,
    )
}
fn nominal(package: &nir::program::Package, name: &str, arguments: Vec<nir::Term>) -> nir::Term {
    nir::Term::Nominal {
        declaration: package
            .structs
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .declaration,
        arguments,
    }
}

#[test]
fn drop_protocol_identity_selects_reference_receiver_and_does_not_hide_field_cleanup() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Drop {fn drop(*self);} struct Owned {} struct Container {value:Owned} struct Pretender {} impl Retire for Owned {fn drop(*self) {let pointer=self;}} impl Drop for Pretender {fn drop(*self) {}}",
    );
    let contract = &package
        .traits
        .iter()
        .find(|t| t.name == "Retire")
        .unwrap()
        .methods[0];
    assert!(contract.by_reference_receiver);
    assert!(!contract.by_value_receiver);
    assert!(
        contract
            .to_string()
            .contains("(by-reference-receiver true)")
    );
    world(&package, |world, items| {
        let result = luna::protocols::select_drop(
            &package,
            world,
            items,
            &nominal(&package, "Owned", vec![]),
            Budget::default(),
        )
        .unwrap();
        let Resolution::Selected(selected) = result else {
            panic!("selected destructor expected")
        };
        assert_eq!(selected.method.member, contract.declaration);
        assert_eq!(
            selected.contract.signature,
            package
                .functions
                .iter()
                .find(|f| f.declaration == selected.method.body)
                .unwrap()
                .signature
        );
        assert!(world.index.type_resolution_is_current(&selected.selection));
        for name in ["Container", "Pretender"] {
            assert!(matches!(
                luna::protocols::select_drop(
                    &package,
                    world,
                    items,
                    &nominal(&package, name, vec![]),
                    Budget::default()
                )
                .unwrap(),
                Resolution::NoCustomDrop(_)
            ));
        }
        assert!(
            luna::protocols::select_drop(
                &package,
                world,
                &LangItems::new(),
                &nominal(&package, "Owned", vec![]),
                Budget::default()
            )
            .is_err()
        );
    });
}

#[test]
fn generic_and_default_destructors_keep_distinct_substitution_plans() {
    let package = compile(
        "trait Retire {fn drop(*self) {let pointer=self;}} struct Box where T {value:T} struct Explicit where T {value:T} impl Retire for Box<T> where T {} impl Retire for Explicit<T> where T {fn drop(*self) {let pointer=self;}}",
    );
    world(&package, |world, items| {
        for name in ["Box", "Explicit"] {
            let argument = nir::Term::Primitive(nir::Primitive::Bool);
            let subject = nominal(&package, name, vec![argument.clone()]);
            let Resolution::Selected(selected) =
                luna::protocols::select_drop(&package, world, items, &subject, Budget::default())
                    .unwrap()
            else {
                panic!("selected")
            };
            assert_eq!(selected.implementation_arguments, [argument.clone()]);
            assert_eq!(
                selected.method.arguments,
                if name == "Box" {
                    vec![subject]
                } else {
                    vec![argument]
                }
            );
            assert_eq!(selected.parameters.len(), 1);
            assert!(selected.to_string().starts_with("(selected-drop"));
        }
    });
}

#[test]
fn destructor_program_specifications_remain_obligations_outside_type_selection() {
    let mut package = compile(
        "trait Retire {fn drop(*self) where ensures true;} struct Owned {} impl Retire for Owned {fn drop(*self) {}}",
    );
    // A normalized program-entry obligation: selection must retain it for the
    // resource verifier, not feed it to the type solver. Source `requires false`
    // without runtime parameters denotes a type predicate in the current IR.
    let mut predicate = package.traits[0].methods[0].program.postconditions[0].clone();
    predicate.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    package.traits[0].methods[0]
        .program
        .preconditions
        .push(predicate.clone());
    package.functions[0].preconditions.push(predicate);
    world(&package, |world, items| {
        let Resolution::Selected(selected) = luna::protocols::select_drop(
            &package,
            world,
            items,
            &nominal(&package, "Owned", vec![]),
            Budget::default(),
        )
        .unwrap() else {
            panic!("program false must not be a type exclusion")
        };
        assert_eq!(selected.contract.program.postconditions.len(), 1);
        assert_eq!(selected.program.postconditions.len(), 1);
        assert_eq!(selected.contract.program.preconditions.len(), 1);
        assert_eq!(selected.program.preconditions.len(), 1);
        assert!(matches!(
            selected.program.preconditions[0].kind,
            nir::program::ExprKind::Literal(nir::program::Literal::Bool(false))
        ));
    });
}

#[test]
fn assumptions_and_exhaustion_never_become_absence() {
    let package = compile(
        "trait Retire {fn drop(*self);} struct Owned {} struct Other {} impl Retire for Owned {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let subject = nominal(&package, "Other", vec![]);
        let protocol = package.traits[0].declaration;
        let assumptions = [nir::program::TraitRequirement {
            interface: protocol,
            interface_arguments: vec![],
            subject: subject.clone(),
        }];
        let given = World {
            assumptions: &assumptions,
            ..*world
        };
        assert!(matches!(
            luna::protocols::select_drop(&package, &given, items, &subject, Budget::default())
                .unwrap(),
            Resolution::Assumed { .. }
        ));
        let unrelated = nominal(&package, "Owned", vec![]);
        let assumptions = [nir::program::TraitRequirement {
            interface: protocol,
            interface_arguments: vec![],
            subject: unrelated,
        }];
        let uncertain = World {
            assumptions: &assumptions,
            ..*world
        };
        assert!(matches!(
            luna::protocols::select_drop(&package, &uncertain, items, &subject, Budget::default())
                .unwrap(),
            Resolution::Pending(_)
        ));
        assert!(matches!(
            luna::protocols::select_drop(
                &package,
                world,
                items,
                &subject,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .unwrap(),
            Resolution::Pending(_)
        ));
        assert!(matches!(
            luna::protocols::select_drop(&package, world, items, &subject, Budget::default())
                .unwrap(),
            Resolution::NoCustomDrop(_)
        ));
    });
}

#[test]
fn type_qualification_is_fixed_with_destructor_selection() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Permit {} struct Allowed {} struct Denied {} struct Box where T {value:T} impl Permit for Allowed {} impl Retire for Box<T> where T :- Permit {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let allowed = nominal(&package, "Allowed", vec![]);
        let subject = nominal(&package, "Box", vec![allowed.clone()]);
        let Resolution::Selected(selected) =
            luna::protocols::select_drop(&package, world, items, &subject, Budget::default())
                .unwrap()
        else {
            panic!("qualified selection")
        };
        assert!(!selected.qualification.is_empty());
        assert_eq!(selected.implementation_arguments, [allowed]);
        let denied = nominal(&package, "Box", vec![nominal(&package, "Denied", vec![])]);
        assert!(matches!(
            luna::protocols::select_drop(&package, world, items, &denied, Budget::default())
                .unwrap(),
            Resolution::NoCustomDrop(_)
        ));
    });
}

#[test]
fn invalid_protocol_and_body_metadata_cannot_authorize_a_destructor() {
    let package = compile(
        "trait Retire {fn drop(*self);} struct Owned {} impl Retire for Owned {fn drop(*self) {}}",
    );
    for mutation in 0..4 {
        let mut invalid = package.clone();
        match mutation {
            0 => invalid.traits[0].methods[0].by_reference_receiver = false,
            1 => {
                invalid.traits[0].methods[0].signature = nir::Term::ComptimeFunction(Box::new(
                    invalid.traits[0].methods[0].signature.clone(),
                ))
            }
            2 => invalid.functions[0].phase = nir::program::FunctionPhase::Comptime,
            _ => invalid.implementations[0].methods.clear(),
        }
        world(&invalid, |world, items| {
            assert!(
                luna::protocols::select_drop(
                    &invalid,
                    world,
                    items,
                    &nominal(&invalid, "Owned", vec![]),
                    Budget::default()
                )
                .is_err(),
                "mutation {mutation}"
            );
        });
    }
    world(&package, |world, items| {
        assert!(matches!(
            luna::protocols::select_drop(
                &package,
                world,
                items,
                &nominal(&package, "Owned", vec![]),
                Budget::default()
            )
            .unwrap(),
            Resolution::Selected(_)
        ));
        let mut other = package.clone();
        other.traits[0].methods[0].by_reference_receiver = false;
        assert!(
            luna::protocols::select_drop(
                &other,
                world,
                items,
                &nominal(&package, "Owned", vec![]),
                Budget::default()
            )
            .is_err()
        );
    });
}

#[test]
fn conflicting_implementations_remain_pending_instead_of_selecting_arbitrarily() {
    let mut package = compile(
        "trait Retire {fn drop(*self);} struct Owned {} impl Retire for Owned {fn drop(*self) {}}",
    );
    let mut duplicate = package.implementations[0].clone();
    duplicate.declaration.local += 10000;
    package.implementations.push(duplicate);
    world(&package, |world, items| {
        let result = luna::protocols::select_drop(
            &package,
            world,
            items,
            &nominal(&package, "Owned", vec![]),
            Budget::default(),
        )
        .unwrap();
        assert!(matches!(result, Resolution::Pending(_)), "{result}");
    });
}

#[test]
fn trait_implementation_receiver_modes_must_match() {
    for source in [
        "trait Retire {fn drop(*self);} struct Owned {} impl Retire for Owned {fn drop(self) {}}",
        "trait Retire {fn drop(self);} struct Owned {} impl Retire for Owned {fn drop(*self) {}}",
        "trait Retire {fn drop(*self);} struct Owned {} impl Retire for Owned {fn drop(value:*Self) {}}",
        "struct Owned {} impl Owned {fn drop(value:bool,*self) {}}",
    ] {
        let path = std::env::temp_dir().join(format!(
            "luna-drop-reject-{}-{}.fl",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, source).unwrap();
        let result = frontend::compile_in_package(
            &Options {
                input: path.clone(),
                emit: Emit::Nir,
                output: None,
                sysroot: None,
                splice_protocol: None,
                meta_list_type: None,
                index_protocol: None, arithmetic_protocols:Default::default(), tuple_type: None, optional_type: None,
            },
            0,
        );
        std::fs::remove_file(path).unwrap();
        assert!(result.is_err(), "receiver mismatch accepted: {source}");
    }
}

#[test]
fn selection_retains_concrete_identity_but_type_evidence_is_world_scoped() {
    let package = compile_in(
        "trait Retire {fn drop(*self);} struct Owned {} impl Retire for Owned {fn drop(*self) {}}",
        77,
    );
    let selected = world(&package, |world, items| {
        let subject = nominal(&package, "Owned", vec![]);
        let Resolution::Selected(selected) =
            luna::protocols::select_drop(&package, world, items, &subject, Budget::default())
                .unwrap()
        else {
            panic!("selected")
        };
        let mut stale = LangItems::new();
        stale
            .set(
                LangItem::Drop,
                intrinsic::LangItemDef::UserDef(nir::DeclarationId {
                    package: 78,
                    ..selected.protocol
                }),
            )
            .unwrap();
        assert!(
            luna::protocols::select_drop(&package, world, &stale, &subject, Budget::default())
                .is_err()
        );
        selected
    });
    assert_eq!(selected.protocol.package, 77);
    assert_eq!(selected.method.body.package, 77);
    world(&package, |world, _| {
        assert!(!world.index.type_resolution_is_current(&selected.selection))
    });
    assert_eq!(
        selected.parameters,
        package
            .functions
            .iter()
            .find(|f| f.declaration == selected.method.body)
            .unwrap()
            .parameters
    );
}

#[test]
fn unestablished_interface_type_requirements_do_not_publish_a_selected_destructor() {
    let mut package = compile(
        "trait Retire {fn drop(*self);} struct Owned {} impl Retire for Owned {fn drop(*self) {}}",
    );
    package.traits[0]
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    world(&package, |world, items| {
        let result = luna::protocols::select_drop(
            &package,
            world,
            items,
            &nominal(&package, "Owned", vec![]),
            Budget::default(),
        )
        .unwrap();
        assert!(matches!(result, Resolution::Pending(_)), "{result}");
    });
    package.traits[0].requirements.clear();
    world(&package, |world, items| {
        assert!(matches!(
            luna::protocols::select_drop(
                &package,
                world,
                items,
                &nominal(&package, "Owned", vec![]),
                Budget::default()
            )
            .unwrap(),
            Resolution::Selected(_)
        ))
    });
}

#[test]
fn captured_drop_access_checks_only_required_remaining_fields() {
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire { fn drop(*self); } struct Ticket {} struct Bundle { kept:bool, moved:Ticket, } impl Retire for Bundle { fn drop(*self) { let flag=self.kept.*; } }",
    );
    world(&package, |world, items| {
        let subject = nominal(&package, "Bundle", vec![]);
        let Resolution::Selected(selected) =
            luna::protocols::select_drop(&package, world, items, &subject, Budget::default())
                .unwrap()
        else {
            panic!("selected")
        };
        let root = PlaceId {
            owner: 100,
            local: 1,
        };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                root,
                &Shape::Fields(vec![Shape::Value, Shape::Fields(vec![])]),
                true,
                &mut 1000,
            )
            .unwrap();
        state
            .take(&Place {
                root,
                fields: vec![1],
            })
            .unwrap();
        let before = state.clone();
        let checked = luna::protocols::check_drop_access(
            &package, world, &selected, &state, &receiver, &mut 10000,
        )
        .unwrap();
        assert_eq!(state, before);
        assert!(
            checked
                .footprint
                .accesses
                .iter()
                .any(|access| access.read_value && access.target.fields[0].1 == 0)
        );
        assert!(checked.to_string().starts_with("(drop-access-check "));
        state
            .take(&Place {
                root,
                fields: vec![0],
            })
            .unwrap();
        assert!(
            luna::protocols::check_drop_access(
                &package, world, &selected, &state, &receiver, &mut 10000
            )
            .unwrap_err()
            .contains("unavailable")
        );
    });
}

#[test]
fn captured_drop_access_distinguishes_parent_construction_from_remaining_children() {
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire { fn drop(*self) {} } struct Pair {left:bool,right:bool,} impl Retire for Pair {}",
    );
    world(&package, |world, items| {
        let Resolution::Selected(selected) = luna::protocols::select_drop(
            &package,
            world,
            items,
            &nominal(&package, "Pair", vec![]),
            Budget::default(),
        )
        .unwrap() else {
            panic!("selected")
        };
        let root = PlaceId {
            owner: 100,
            local: 1,
        };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let shape = Shape::Fields(vec![Shape::Value, Shape::Value]);
        let mut state = State::default();
        state.declare(root, &shape, true, &mut 1000).unwrap();
        for index in 0..2 {
            state
                .take(&Place {
                    root,
                    fields: vec![index],
                })
                .unwrap();
        }
        assert!(
            luna::protocols::check_drop_access(
                &package, world, &selected, &state, &receiver, &mut 10000
            )
            .unwrap()
            .footprint
            .accesses
            .is_empty()
        );
        let mut absent = State::default();
        absent.declare(root, &shape, false, &mut 1000).unwrap();
        assert!(
            luna::protocols::check_drop_access(
                &package, world, &selected, &absent, &receiver, &mut 10000
            )
            .is_err()
        );
        let conditional = state.join(&absent, &mut 1000).unwrap();
        assert!(
            luna::protocols::check_drop_access(
                &package,
                world,
                &selected,
                &conditional,
                &receiver,
                &mut 10000
            )
            .unwrap_err()
            .contains("guard")
        );
        let mut incomplete = State::default();
        incomplete.declare(root, &shape, false, &mut 1000).unwrap();
        incomplete
            .install(&Place {
                root,
                fields: vec![0],
            })
            .unwrap();
        assert!(
            luna::protocols::check_drop_access(
                &package,
                world,
                &selected,
                &incomplete,
                &receiver,
                &mut 10000
            )
            .unwrap_err()
            .contains("construction")
        );
    });
}

#[test]
fn captured_drop_access_specializes_and_rebases_subobject_footprints() {
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Box where T {flag:bool,value:T,} fn update(flag:*bool) { flag.* = true; } impl Retire for Box<T> where T {fn drop(*self) {update(self.flag);}}",
    );
    world(&package, |world, items| {
        let Resolution::Selected(selected) = luna::protocols::select_drop(
            &package,
            world,
            items,
            &nominal(
                &package,
                "Box",
                vec![nir::Term::Primitive(nir::Primitive::Char)],
            ),
            Budget::default(),
        )
        .unwrap() else {
            panic!("selected")
        };
        let root = PlaceId {
            owner: 100,
            local: 1,
        };
        let receiver = Place {
            root,
            fields: vec![0],
        };
        let mut state = State::default();
        state
            .declare(
                root,
                &Shape::Fields(vec![
                    Shape::Fields(vec![Shape::Value, Shape::Value]),
                    Shape::Value,
                ]),
                true,
                &mut 1000,
            )
            .unwrap();
        state
            .take(&Place {
                root,
                fields: vec![0, 1],
            })
            .unwrap();
        assert!(
            luna::protocols::check_drop_access(
                &package, world, &selected, &state, &receiver, &mut 0
            )
            .is_err()
        );
        let checked = luna::protocols::check_drop_access(
            &package, world, &selected, &state, &receiver, &mut 10000,
        )
        .unwrap();
        assert!(checked.footprint.accesses.iter().any(|access| access.write));
        let mut damaged = selected.clone();
        damaged.method.arguments.clear();
        assert!(
            luna::protocols::check_drop_access(
                &package, world, &damaged, &state, &receiver, &mut 10000
            )
            .is_err()
        );
    });
}

#[test]
fn captured_drop_access_rejects_stale_world_and_changed_layout_metadata() {
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Cell {flag:bool,} impl Retire for Cell {fn drop(*self) {let value=self.flag.*;}}",
    );
    world(&package, |original, items| {
        let Resolution::Selected(selected) = luna::protocols::select_drop(
            &package,
            original,
            items,
            &nominal(&package, "Cell", vec![]),
            Budget::default(),
        )
        .unwrap() else {
            panic!("selected")
        };
        let root = PlaceId {
            owner: 100,
            local: 1,
        };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(root, &Shape::Fields(vec![Shape::Value]), true, &mut 1000)
            .unwrap();
        world(&package, |other, _| {
            assert!(
                luna::protocols::check_drop_access(
                    &package, other, &selected, &state, &receiver, &mut 10000
                )
                .unwrap_err()
                .contains("different implementation world")
            );
        });
        let mut damaged = package.clone();
        damaged
            .structs
            .iter_mut()
            .find(|s| s.name == "Cell")
            .unwrap()
            .fields[0]
            .1 = nir::Term::Primitive(nir::Primitive::Char);
        assert!(
            luna::protocols::check_drop_access(
                &damaged, original, &selected, &state, &receiver, &mut 10000
            )
            .unwrap_err()
            .contains("type disagrees")
        );
        let mut changed = package.clone();
        changed
            .functions
            .iter_mut()
            .find(|f| f.declaration == selected.method.body)
            .unwrap()
            .preconditions
            .push(nir::program::Expr {
                source: nir::program::SourceRange { start: 0, end: 0 },
                ty: nir::Term::Primitive(nir::Primitive::Bool),
                kind: nir::program::ExprKind::Literal(nir::program::Literal::Bool(false)),
            });
        assert!(
            luna::protocols::check_drop_access(
                &changed, original, &selected, &state, &receiver, &mut 10000
            )
            .unwrap_err()
            .contains("contract changed")
        );
        assert!(
            luna::protocols::check_drop_access(
                &package, original, &selected, &state, &receiver, &mut 10000
            )
            .is_ok()
        );
    });
}

#[test]
fn captured_drop_access_distinguishes_pointer_representation_from_pointee() {
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    for (body, accepted) in [
        ("let pointer=self.pointer.*; let copy=pointer;", true),
        ("let value=self.pointer.*.*;", false),
    ] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} struct Observer {{pointer:*bool,}} impl Retire for Observer {{fn drop(*self) {{{body}}}}}"
        ));
        world(&package, |world, items| {
            let Resolution::Selected(selected) = luna::protocols::select_drop(
                &package,
                world,
                items,
                &nominal(&package, "Observer", vec![]),
                Budget::default(),
            )
            .unwrap() else {
                panic!("selected")
            };
            let root = PlaceId {
                owner: 100,
                local: 1,
            };
            let receiver = Place {
                root,
                fields: vec![],
            };
            let mut state = State::default();
            state
                .declare(root, &Shape::Fields(vec![Shape::Value]), true, &mut 1000)
                .unwrap();
            // The pointer slot exists; no pointee lifetime/access evidence was
            // supplied. Copying that representation must not invent such proof.
            let result = luna::protocols::check_drop_access(
                &package, world, &selected, &state, &receiver, &mut 10000,
            );
            assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        });
    }
}

#[test]
fn acquired_cleanup_plans_parent_drop_then_reverse_fields_without_reselection() {
    use elaboration::verification::{drop::{plan_cleanup,CleanupStep},ownership::{State,Shape,Place},PlaceId};
    let package=compile("trait Retire {fn drop(*self);} struct Left {} struct Right {} struct Parent {left:Left,right:Right,} impl Retire for Left {fn drop(*self) {}} impl Retire for Right {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {}}");
    world(&package,|world,items| {
        let acquired=luna::protocols::acquire_cleanup(&package,world,items,&nominal(&package,"Parent",vec![]),Budget::default(),&mut 10000).unwrap();
        let root=PlaceId {owner:1,local:1};let receiver=Place {root,fields:vec![]};
        let mut state=State::default();
        state.declare(root,&Shape::Fields(vec![Shape::Fields(vec![]),Shape::Fields(vec![])]),true,&mut 1000).unwrap();
        let original=state.clone();
        let plan=plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).unwrap();
        let paths:Vec<_>=plan.steps.iter().filter_map(|s| if let CleanupStep::Invoke {access,..}=s {Some(access.receiver.fields.clone())}else{None}).collect();
        assert_eq!(paths,vec![vec![],vec![1],vec![0]]);
        assert_eq!(plan.steps.len(),6);
        assert_eq!(state,original);
        assert!(acquired.to_string().starts_with("(acquired-cleanup "));
        assert!(plan.to_string().starts_with("(cleanup-plan "));
        println!("CLEANUP_PLAN_DUMP={plan}");
        state.take(&Place {root,fields:vec![1]}).unwrap();
        let plan=plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).unwrap();
        let paths:Vec<_>=plan.steps.iter().filter_map(|s| if let CleanupStep::Invoke {access,..}=s {Some(access.receiver.fields.clone())}else{None}).collect();
        assert_eq!(paths,vec![vec![],vec![0]]);
        state.take(&Place {root,fields:vec![0]}).unwrap();
        assert_eq!(plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).unwrap().steps.len(),2);
    });
}

#[test]
fn acquired_cleanup_skips_unconstructed_parents_and_moved_positions() {
    use elaboration::verification::{drop::{acquire_cleanup,plan_cleanup,CleanupStep},ownership::{State,Shape,Place},PlaceId};
    let package=compile("trait Retire {fn drop(*self);} struct Child {} struct Parent {first:Child,second:Child,} impl Retire for Child {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {}}");
    world(&package,|world,_| {
        let acquired=acquire_cleanup(&package,world,package.traits[0].declaration,&nominal(&package,"Parent",vec![]),Budget::default(),&mut 10000).unwrap();
        let root=PlaceId {owner:1,local:1};let receiver=Place {root,fields:vec![]};
        let mut state=State::default();
        let shape=Shape::Fields(vec![Shape::Fields(vec![]),Shape::Fields(vec![])]);
        state.declare(root,&shape,false,&mut 1000).unwrap();
        state.install(&Place {root,fields:vec![0]}).unwrap();
        let plan=plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).unwrap();
        assert_eq!(plan.steps.len(),2,"{plan}");
        assert!(matches!(&plan.steps[0],CleanupStep::Invoke {access,..} if access.receiver.fields==[0]));
        let mut complete=State::default();complete.declare(root,&shape,true,&mut 1000).unwrap();
        complete.take(&receiver).unwrap();
        assert!(plan_cleanup(&package,world,&acquired,&complete,&receiver,&mut 10000).unwrap().steps.is_empty());
    });
}

#[test]
fn acquired_cleanup_uses_construction_implications_and_rejects_missing_fields() {
    use elaboration::verification::{drop::{acquire_cleanup,plan_cleanup},ownership::{State,Shape,Place},PlaceId};
    let package=compile("trait Retire {fn drop(*self);} struct Cell {value:bool,} impl Retire for Cell {fn drop(*self) {let seen=self.value.*;}}");
    world(&package,|world,_| {
        let acquired=acquire_cleanup(&package,world,package.traits[0].declaration,&nominal(&package,"Cell",vec![]),Budget::default(),&mut 10000).unwrap();
        let root=PlaceId {owner:1,local:1};let receiver=Place {root,fields:vec![]};
        let mut state=State::default();state.declare(root,&Shape::Fields(vec![Shape::Value]),true,&mut 1000).unwrap();
        assert!(plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).is_ok());
        let mut absent=state.clone();absent.take(&receiver).unwrap();
        let conditional=state.join(&absent,&mut 1000).unwrap();
        assert!(plan_cleanup(&package,world,&acquired,&conditional,&receiver,&mut 10000).is_ok());
        state.take(&Place {root,fields:vec![0]}).unwrap();
        let before=state.clone();
        let conditional_partial=state.join(&absent,&mut 1000).unwrap();
        assert!(plan_cleanup(&package,world,&acquired,&conditional_partial,&receiver,&mut 10000).unwrap_err().contains("guard"));
        assert!(plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).unwrap_err().contains("unavailable"));
        assert_eq!(state,before);
    });
}

#[test]
fn acquired_cleanup_rejects_changed_world_layout_and_exhausted_work() {
    use elaboration::verification::{drop::{acquire_cleanup,plan_cleanup},ownership::{State,Shape,Place},PlaceId};
    let package=compile("trait Retire {fn drop(*self);} struct Plain {}");
    world(&package,|first,_| {
        let ty=nominal(&package,"Plain",vec![]);
        assert!(acquire_cleanup(&package,first,package.traits[0].declaration,&ty,Budget::default(),&mut 0).is_err());
        let acquired=acquire_cleanup(&package,first,package.traits[0].declaration,&ty,Budget::default(),&mut 10000).unwrap();
        let root=PlaceId {owner:1,local:1};let receiver=Place {root,fields:vec![]};
        let mut state=State::default();state.declare(root,&Shape::Fields(vec![]),true,&mut 1000).unwrap();
        assert_eq!(plan_cleanup(&package,first,&acquired,&state,&receiver,&mut 10000).unwrap().steps.len(),1);
        assert!(plan_cleanup(&package,first,&acquired,&state,&receiver,&mut 0).is_err());
        world(&package,|second,_| assert!(plan_cleanup(&package,second,&acquired,&state,&receiver,&mut 10000).unwrap_err().contains("different implementation world")));
        let mut changed=package.clone();changed.structs[0].fields.push(("later".into(),nir::Term::Primitive(nir::Primitive::Bool)));
        assert!(plan_cleanup(&changed,first,&acquired,&state,&receiver,&mut 10000).unwrap_err().contains("layout changed"));
        assert!(plan_cleanup(&package,first,&acquired,&state,&receiver,&mut 10000).is_ok());
    });
}

#[test]
fn acquired_cleanup_rejects_unbounded_types_and_wrong_ownership_shapes() {
    use elaboration::verification::{drop::{acquire_cleanup,plan_cleanup},ownership::{State,Shape,Place},PlaceId};
    let package=compile("trait Retire {fn drop(*self);} struct Plain {}");
    world(&package,|world,items| {
        let ty=nominal(&package,"Plain",vec![]);
        let mut deep=ty.clone();
        for _ in 0..128 {deep=nir::Term::Pointer(Box::new(deep));}
        assert!(acquire_cleanup(&package,world,package.traits[0].declaration,&deep,Budget::default(),&mut 10000).unwrap_err().contains("type budget"));
        assert!(luna::protocols::acquire_cleanup(&package,world,&LangItems::new(),&ty,Budget::default(),&mut 1000).unwrap_err().contains("not bound"));
        let acquired=luna::protocols::acquire_cleanup(&package,world,items,&ty,Budget::default(),&mut 10000).unwrap();
        let root=PlaceId {owner:1,local:1};let receiver=Place {root,fields:vec![]};
        let mut state=State::default();state.declare(root,&Shape::Value,true,&mut 1000).unwrap();
        assert!(plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).unwrap_err().contains("ownership shape"));
    });
}

#[test]
fn enum_cleanup_dispatches_only_the_active_payload_in_reverse_order() {
    use elaboration::verification::{
        PlaceId,
        drop::{CleanupStep, plan_cleanup, plan_cleanup_with_variants},
        moves::VariantSelection,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} impl Retire for Ticket {fn drop(*self) {}} impl Retire for Choice {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let ty = nir::Term::Nominal {
            declaration: package.enums[0].declaration,
            arguments: vec![],
        };
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &ty,
            Budget::default(),
            &mut 10000,
        )
        .unwrap();
        let root = PlaceId { owner: 4, local: 1 };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let mut state = State::default();
        state.declare(root, &Shape::Value, true, &mut 1000).unwrap();
        let before = state.clone();
        let plan = plan_cleanup(&package, world, &acquired, &state, &receiver, &mut 10000).unwrap();
        assert_eq!(plan.steps.len(), 3, "{plan}");
        assert!(
            matches!(&plan.steps[0],CleanupStep::Invoke {access,..} if access.receiver==receiver)
        );
        let CleanupStep::SwitchVariant { place, branches } = &plan.steps[1] else {
            panic!("{plan}")
        };
        assert_eq!(place, &receiver);
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].index, 0);
        assert!(branches[0].steps.is_empty());
        assert_eq!(branches[1].index, 1);
        let paths: Vec<_> = branches[1]
            .steps
            .iter()
            .filter_map(|s| match s {
                CleanupStep::Invoke { access, .. } => Some(access.receiver.fields.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(paths, vec![vec![1], vec![0]]);
        assert_eq!(branches[1].steps.len(), 4);
        assert!(matches!(&plan.steps[2],CleanupStep::Release {place,..} if place==&receiver));
        println!("ENUM_CLEANUP_DUMP={plan}");
        for (index, count) in [(0, 2), (1, 6)] {
            let evidence = [VariantSelection {
                fields: vec![],
                index,
            }];
            let selected = plan_cleanup_with_variants(
                &package, world, &acquired, &state, &receiver, &evidence, &mut 10000,
            )
            .unwrap();
            assert_eq!(selected.steps.len(), count);
            assert!(
                !selected
                    .steps
                    .iter()
                    .any(|s| matches!(s, CleanupStep::SwitchVariant { .. }))
            );
        }
        assert_eq!(state, before);
        let mut changed = package.clone();
        changed.enums[0].variants[1].fields.pop();
        assert!(
            plan_cleanup(&changed, world, &acquired, &state, &receiver, &mut 10000)
                .unwrap_err()
                .contains("layout changed")
        );
        assert!(plan_cleanup(&package, world, &acquired, &state, &receiver, &mut 0).is_err());
        assert!(plan_cleanup(&package, world, &acquired, &state, &receiver, &mut 10000).is_ok());
    });
}

#[test]
fn enum_cleanup_preserves_partial_payload_obligations_without_guessing_a_tag() {
    use elaboration::verification::{
        PlaceId,
        drop::{CleanupStep, plan_cleanup, plan_cleanup_with_variants},
        moves::VariantSelection,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} impl Retire for Ticket {fn drop(*self) {}} impl Retire for Choice {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let ty = nir::Term::Nominal {
            declaration: package.enums[0].declaration,
            arguments: vec![],
        };
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &ty,
            Budget::default(),
            &mut 10000,
        )
        .unwrap();
        let root = PlaceId { owner: 5, local: 1 };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                root,
                &Shape::Fields(vec![Shape::Fields(vec![]), Shape::Fields(vec![])]),
                true,
                &mut 1000,
            )
            .unwrap();
        state
            .take(&Place {
                root,
                fields: vec![1],
            })
            .unwrap();
        let before = state.clone();
        assert!(
            plan_cleanup(&package, world, &acquired, &state, &receiver, &mut 10000)
                .unwrap_err()
                .contains("requires active variant")
        );
        let evidence = [VariantSelection {
            fields: vec![],
            index: 1,
        }];
        let plan = plan_cleanup_with_variants(
            &package, world, &acquired, &state, &receiver, &evidence, &mut 10000,
        )
        .unwrap();
        let paths: Vec<_> = plan
            .steps
            .iter()
            .filter_map(|s| match s {
                CleanupStep::Invoke { access, .. } => Some(access.receiver.fields.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(paths, vec![vec![], vec![0]]);
        assert_eq!(plan.steps.len(), 4);
        for bad in [
            vec![VariantSelection {
                fields: vec![],
                index: 2,
            }],
            vec![evidence[0].clone(), evidence[0].clone()],
            vec![
                VariantSelection {
                    fields: vec![0],
                    index: 0,
                },
                evidence[0].clone(),
            ],
        ] {
            assert!(
                plan_cleanup_with_variants(
                    &package, world, &acquired, &state, &receiver, &bad, &mut 10000
                )
                .is_err()
            );
        }
        assert_eq!(state, before);
        // Even when all payloads have moved, a tag for another layout must not
        // make the retained aggregate construction marker look well typed.
        state
            .take(&Place {
                root,
                fields: vec![0],
            })
            .unwrap();
        assert!(
            plan_cleanup_with_variants(
                &package,
                world,
                &acquired,
                &state,
                &receiver,
                &[VariantSelection {
                    fields: vec![],
                    index: 0
                }],
                &mut 10000
            )
            .unwrap_err()
            .contains("ownership shape")
        );
        let exhausted = plan_cleanup_with_variants(
            &package, world, &acquired, &state, &receiver, &evidence, &mut 10000,
        )
        .unwrap();
        assert_eq!(exhausted.steps.len(), 2);
    });
}

#[test]
fn enum_cleanup_nested_generic_payloads_require_correlated_evidence() {
    use elaboration::verification::{
        PlaceId,
        drop::{CleanupStep, plan_cleanup, plan_cleanup_with_variants},
        moves::VariantSelection,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Ticket {} enum Maybe where T {none,some(T)} struct Holder {item:Maybe<Maybe<Ticket>>,} impl Retire for Ticket {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &nominal(&package, "Holder", vec![]),
            Budget::default(),
            &mut 20000,
        )
        .unwrap();
        let root = PlaceId { owner: 6, local: 1 };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(root, &Shape::Fields(vec![Shape::Value]), true, &mut 1000)
            .unwrap();
        let plan = plan_cleanup(&package, world, &acquired, &state, &receiver, &mut 20000).unwrap();
        let CleanupStep::SwitchVariant { branches, .. } = &plan.steps[0] else {
            panic!("{plan}")
        };
        assert!(
            matches!(&branches[1].steps[0],CleanupStep::SwitchVariant {place,..} if place.fields==[0,0])
        );
        let nested = VariantSelection {
            fields: vec![0, 0],
            index: 1,
        };
        assert!(
            plan_cleanup_with_variants(
                &package,
                world,
                &acquired,
                &state,
                &receiver,
                &[nested.clone()],
                &mut 20000
            )
            .unwrap_err()
            .contains("enclosing variant")
        );
        let evidence = [
            nested,
            VariantSelection {
                fields: vec![0],
                index: 1,
            },
        ];
        let selected = plan_cleanup_with_variants(
            &package, world, &acquired, &state, &receiver, &evidence, &mut 20000,
        )
        .unwrap();
        assert_eq!(selected.steps.len(), 5, "{selected}");
        assert!(
            matches!(&selected.steps[0],CleanupStep::Invoke {access,..} if access.receiver.fields==[0,0,0])
        );
        assert!(
            !selected
                .steps
                .iter()
                .any(|s| matches!(s, CleanupStep::SwitchVariant { .. }))
        );
    });
}

fn cleanup_report(
    package: &nir::program::Package,
    name: &str,
) -> Result<elaboration::verification::moves::Report, elaboration::verification::moves::Error> {
    world(package, |world, _| {
        let mut items = LangItems::new();
        items
            .bind_source_traits(
                package,
                &[
                    (
                        LangItem::Drop,
                        package
                            .traits
                            .iter()
                            .find(|t| t.name == "Retire")
                            .unwrap()
                            .declaration,
                    ),
                    (
                        LangItem::Copy,
                        package
                            .traits
                            .iter()
                            .find(|t| t.name == "Duplicate")
                            .unwrap()
                            .declaration,
                    ),
                ],
            )
            .unwrap();
        luna::protocols::analyze_cleanup(
            package,
            world,
            &items,
            package.functions.iter().find(|f| f.name == name).unwrap(),
            100000,
            Budget::default(),
        )
    })
}

#[test]
fn source_cleanup_follows_moved_owners_and_rejects_invalid_destructor_access() {
    use elaboration::verification::{drop::CleanupStep, moves::CleanupAction};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} impl Retire for Ticket {fn drop(*self) {}} fn run() {let first=Ticket {};let second=first;} fn returned()->Ticket {Ticket {}}",
    );
    let report = cleanup_report(&package, "run").unwrap();
    let actions: Vec<_> = report
        .cleanup_sites
        .iter()
        .flat_map(|s| &s.actions)
        .collect();
    assert_eq!(actions.len(), 1, "{report}");
    let CleanupAction::Planned { candidate, plan } = actions[0] else {
        panic!("{report}")
    };
    assert!(matches!(candidate.as_ref(), CleanupAction::Owned { .. }));
    assert_eq!(plan.steps.len(), 2);
    assert!(matches!(&plan.steps[0], CleanupStep::Invoke { .. }));
    assert!(
        cleanup_report(&package, "returned")
            .unwrap()
            .cleanup_sites
            .is_empty()
    );
    println!("SOURCE_CLEANUP_DUMP={report}");
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {flag:bool,} struct Holder {child:Ticket,} impl Retire for Holder {fn drop(*self) {let observed=self.child.flag.*;}} fn bad() {let parent=Holder {child:Ticket {flag:true}};let extracted=parent.child;}",
    );
    assert!(
        cleanup_report(&package, "bad")
            .unwrap_err()
            .reason
            .contains("unavailable")
    );
}

#[test]
fn source_cleanup_plans_temporaries_prepared_values_and_replacements() {
    use elaboration::verification::{
        drop::CleanupStep,
        moves::{CleanupAction, CleanupReason},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} struct Pair {first:Ticket,second:Ticket,} impl Retire for Ticket {fn drop(*self) {}} fn temporary() {Ticket {};} fn prepare() {let pair=Pair {first:Ticket {},second:do {return;Ticket {}}};} fn replace() {var ticket=Ticket {};ticket=Ticket {};}",
    );
    for (entry, reason, kind, count) in [
        ("temporary", CleanupReason::TemporaryEnd, 0, 1),
        ("prepare", CleanupReason::Return, 1, 1),
        ("replace", CleanupReason::Replacement, 2, 2),
    ] {
        let report = cleanup_report(&package, entry).unwrap();
        let actions: Vec<_> = report
            .cleanup_sites
            .iter()
            .flat_map(|s| &s.actions)
            .collect();
        assert_eq!(actions.len(), count, "{entry}: {report}");
        let site = report
            .cleanup_sites
            .iter()
            .find(|s| s.reason == reason)
            .unwrap();
        let CleanupAction::Planned { candidate, plan } = &site.actions[0] else {
            panic!("{report}")
        };
        assert!(match kind {
            0 => matches!(candidate.as_ref(), CleanupAction::Temporary { .. }),
            1 => matches!(candidate.as_ref(), CleanupAction::Prepared { .. }),
            _ => matches!(candidate.as_ref(), CleanupAction::Owned { .. }),
        });
        assert_eq!(plan.steps.len(), 2, "{plan}");
        assert!(matches!(&plan.steps[0], CleanupStep::Invoke { .. }));
    }
}

#[test]
fn source_cleanup_keeps_enum_decomposition_and_failure_paths() {
    use elaboration::verification::{
        drop::CleanupStep,
        moves::{CleanupAction, CleanupReason},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} enum Choice {none,pair(Ticket,Ticket)} impl Retire for Ticket {fn drop(*self) {}} fn selected(value:Choice) {let .pair(first,_)=value else {return;};} fn failed(flag:bool) {let held=Ticket {};assumes flag;}",
    );
    let report = cleanup_report(&package, "selected").unwrap();
    let mut payload_plan = false;
    for action in report.cleanup_sites.iter().flat_map(|s| &s.actions) {
        let CleanupAction::Planned { candidate, plan } = action else {
            panic!("{report}")
        };
        if let CleanupAction::Temporary { variants, .. } = candidate.as_ref() {
            if !variants.is_empty() {
                assert!(
                    !plan
                        .steps
                        .iter()
                        .any(|s| matches!(s, CleanupStep::SwitchVariant { .. }))
                );
                payload_plan = true;
            }
        }
    }
    assert!(payload_plan, "{report}");
    let report = cleanup_report(&package, "failed").unwrap();
    for reason in [CleanupReason::PredicateFailure, CleanupReason::ScopeEnd] {
        let site = report
            .cleanup_sites
            .iter()
            .find(|s| s.reason == reason)
            .unwrap();
        assert_eq!(site.actions.iter().filter(|a| matches!(a,CleanupAction::Planned {plan,..} if plan.steps.iter().any(|s| matches!(s,CleanupStep::Invoke {..})))).count(),1,"{report}");
    }
}

#[test]
fn source_cleanup_nested_closures_loops_and_defaults_keep_plans_local() {
    use elaboration::verification::moves::{CleanupAction, CleanupReason, Report};
    fn check(report: &Report) -> usize {
        let mut count = 0;
        for action in report.cleanup_sites.iter().flat_map(|s| &s.actions) {
            let CleanupAction::Planned { candidate, plan } = action else {
                panic!("unplanned: {report}")
            };
            assert!(!matches!(candidate.as_ref(), CleanupAction::Planned { .. }));
            assert!(!plan.steps.is_empty());
            count += 1;
        }
        for child in &report.closures {
            count += check(&child.invocation);
        }
        for child in &report.defaults {
            count += check(&child.evaluation);
        }
        count
    }
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} impl Retire for Ticket {fn drop(*self) {}} fn consume(ticket:Ticket,.enabled:bool=true) {} fn nested() {let held=Ticket {};let action=|| {let moved=held;};action();} fn looped() {while true {let inner=Ticket {};break;}} fn defaults() {consume(Ticket {});}",
    );
    for entry in ["nested", "looped", "defaults"] {
        let report = cleanup_report(&package, entry).unwrap();
        let count = check(&report);
        if entry == "nested" {
            assert!(!report.closures.is_empty());
            assert!(count > 0);
        }
        if entry == "looped" {
            assert_eq!(
                report
                    .cleanup_sites
                    .iter()
                    .filter(|s| s.reason == CleanupReason::Break)
                    .count(),
                1,
                "{report}"
            );
        }
        if entry == "defaults" {
            assert_eq!(report.defaults.len(), 1);
        }
    }
}

#[test]
fn cleanup_session_never_selects_during_planning_and_reuses_captured_fields() {
    use elaboration::verification::{
        PlaceId,
        drop::CleanupSession,
        moves::CleanupPolicy,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Ticket {} struct Holder {ticket:Ticket,} impl Retire for Ticket {fn drop(*self) {}}",
    );
    world(&package, |world, _| {
        let mut session = CleanupSession::new(
            &package,
            world,
            package.traits[0].declaration,
            Budget::default(),
        );
        let root = PlaceId {
            owner: 100,
            local: 1,
        };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(root, &Shape::Fields(vec![]), true, &mut 1000)
            .unwrap();
        let ticket = nominal(&package, "Ticket", vec![]);
        assert!(
            session
                .plan(&ticket, &state, &receiver, &[], &mut 10000)
                .unwrap_err()
                .contains("no acquired binding")
        );
        assert!(
            session
                .acquire(&nominal(&package, "Holder", vec![]), &mut 0)
                .is_err()
        );
        assert!(
            session
                .plan(&ticket, &state, &receiver, &[], &mut 10000)
                .is_err()
        );
        session
            .acquire(&nominal(&package, "Holder", vec![]), &mut 10000)
            .unwrap();
        // No separate Ticket acquisition is needed when its responsibility moves
        // out of Holder: its destructor was captured with Holder's field tree.
        let plan = session
            .plan(&ticket, &state, &receiver, &[], &mut 10000)
            .unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert!(
            session
                .plan(&ticket, &state, &receiver, &[], &mut 0)
                .is_err()
        );
        assert!(
            session
                .plan(&ticket, &state, &receiver, &[], &mut 10000)
                .is_ok()
        );
    });
}

#[test]
fn cleanup_execution_calls_parent_then_fields_on_the_same_storage() {
    use comptime_interpretor::{Event, Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Left {flag:bool,} struct Right {flag:bool,} struct Parent {left:Left,right:Right,} impl Retire for Left {fn drop(*self) {let flag=self.flag.*;assumes not flag;}} impl Retire for Right {fn drop(*self) {let flag=self.flag.*;assumes not flag;}} impl Retire for Parent {fn drop(*self) {self.left.flag.* = false;self.right.flag.* = false;}}",
    );
    let record = |name: &str, fields| {
        let nir::Term::Nominal {
            declaration,
            arguments,
        } = nominal(&package, name, vec![])
        else {
            unreachable!()
        };
        Value::Record {
            declaration,
            arguments,
            fields,
        }
    };
    world(&package, |world, items| {
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &nominal(&package, "Parent", vec![]),
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 70,
                local: 1,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                receiver.root,
                &Shape::Fields(vec![
                    Shape::Fields(vec![Shape::Value]),
                    Shape::Fields(vec![Shape::Value]),
                ]),
                true,
                &mut 1000,
            )
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        println!("EXECUTABLE_CLEANUP_DUMP={prepared}\nEXECUTABLE_CLEANUP_END");
        let storage = Storage::new(record(
            "Parent",
            vec![
                record("Left", vec![Value::Bool(true)]),
                record("Right", vec![Value::Bool(true)]),
            ],
        ));
        let mut interpreter = Interpreter::from_iir(&prepared.graph, Default::default()).unwrap();
        interpreter
            .execute_cleanup(&prepared.program, &storage)
            .unwrap();
        assert_eq!(storage.read().unwrap(), Value::Unit);
        let calls: Vec<_> = interpreter
            .events()
            .iter()
            .filter_map(|e| {
                if let Event::EnterFunction(id) = e {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        let bodies: Vec<_> = prepared
            .program
            .steps
            .iter()
            .filter_map(|s| {
                if let iir::cleanup::Step::Invoke { method, .. } = s {
                    Some(method.declaration)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(calls, bodies);
        assert_eq!(calls.len(), 3);
        assert_eq!(
            interpreter
                .events()
                .iter()
                .filter(|e| matches!(e, Event::CheckPredicate { passed: true, .. }))
                .count(),
            2
        );
        assert!(
            interpreter
                .execute_cleanup(&prepared.program, &storage)
                .is_err(),
            "released storage cannot be cleaned again"
        );
    });
}

#[test]
fn cleanup_execution_dispatches_generic_enum_and_tuple_payloads() {
    use comptime_interpretor::{Event, Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Ticket {} enum Choice where T {empty,pair(T,T)} impl Retire for Ticket {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let ticket = nominal(&package, "Ticket", vec![]);
        let nir::Term::Nominal {
            declaration: ticket_id,
            ..
        } = ticket.clone()
        else {
            unreachable!()
        };
        let enum_ty = nir::Term::Nominal {
            declaration: package.enums[0].declaration,
            arguments: vec![ticket.clone()],
        };
        let ty = nir::Term::Tuple(vec![enum_ty.clone(), ticket.clone()]);
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &ty,
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 71,
                local: 1,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                receiver.root,
                &Shape::Fields(vec![Shape::Value, Shape::Fields(vec![])]),
                true,
                &mut 1000,
            )
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        for index in [0, 1] {
            let value = || Value::Record {
                declaration: ticket_id,
                arguments: vec![],
                fields: vec![],
            };
            let storage = Storage::new(Value::Tuple(vec![
                Value::Variant {
                    declaration: package.enums[0].declaration,
                    arguments: vec![ticket.clone()],
                    index,
                    fields: if index == 0 {
                        vec![]
                    } else {
                        vec![value(), value()]
                    },
                },
                value(),
            ]));
            let mut interpreter =
                Interpreter::from_iir(&prepared.graph, Default::default()).unwrap();
            interpreter
                .execute_cleanup(&prepared.program, &storage)
                .unwrap();
            assert_eq!(storage.read().unwrap(), Value::Unit);
            assert_eq!(
                interpreter
                    .events()
                    .iter()
                    .filter(|e| matches!(e, Event::EnterFunction(_)))
                    .count(),
                if index == 0 { 1 } else { 3 }
            );
        }
    });
}

#[test]
fn cleanup_execution_stops_on_rejection_and_keeps_completed_writes() {
    use comptime_interpretor::{ErrorKind, Event, Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Failing {flag:bool,} struct Later {} struct Parent {later:Later,failing:Failing,} impl Retire for Later {fn drop(*self) {}} impl Retire for Failing {fn drop(*self) {self.flag.* = false;assumes false;}}",
    );
    let record = |name: &str, fields| {
        let nir::Term::Nominal {
            declaration,
            arguments,
        } = nominal(&package, name, vec![])
        else {
            unreachable!()
        };
        Value::Record {
            declaration,
            arguments,
            fields,
        }
    };
    world(&package, |world, items| {
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &nominal(&package, "Parent", vec![]),
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 72,
                local: 1,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                receiver.root,
                &Shape::Fields(vec![
                    Shape::Fields(vec![]),
                    Shape::Fields(vec![Shape::Value]),
                ]),
                true,
                &mut 1000,
            )
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        let storage = Storage::new(record(
            "Parent",
            vec![
                record("Later", vec![]),
                record("Failing", vec![Value::Bool(true)]),
            ],
        ));
        let mut interpreter = Interpreter::from_iir(&prepared.graph, Default::default()).unwrap();
        assert_eq!(
            interpreter
                .execute_cleanup(&prepared.program, &storage)
                .unwrap_err()
                .kind,
            ErrorKind::ProgramContractRejected
        );
        assert_eq!(
            interpreter
                .events()
                .iter()
                .filter(|e| matches!(e, Event::EnterFunction(_)))
                .count(),
            1
        );
        assert_eq!(
            storage.read().unwrap(),
            record(
                "Parent",
                vec![
                    record("Later", vec![]),
                    record("Failing", vec![Value::Bool(false)])
                ]
            )
        );
    });
}

#[test]
fn cleanup_execution_checks_selected_tags_before_any_destructor() {
    use comptime_interpretor::{ErrorKind, Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        moves::VariantSelection,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Left {} struct Right {} enum Choice {left(Left),right(Right)} impl Retire for Left {fn drop(*self) {}} impl Retire for Right {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let ty = nir::Term::Nominal {
            declaration: package.enums[0].declaration,
            arguments: vec![],
        };
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &ty,
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 73,
                local: 1,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(receiver.root, &Shape::Value, true, &mut 1000)
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[VariantSelection {
                fields: vec![],
                index: 1,
            }],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        for (index, name) in [(0, "Left"), (1, "Right")] {
            let nir::Term::Nominal {
                declaration,
                arguments,
            } = nominal(&package, name, vec![])
            else {
                unreachable!()
            };
            let value = Value::Variant {
                declaration: package.enums[0].declaration,
                arguments: vec![],
                index,
                fields: vec![Value::Record {
                    declaration,
                    arguments,
                    fields: vec![],
                }],
            };
            let storage = Storage::new(value.clone());
            let mut interpreter =
                Interpreter::from_iir(&prepared.graph, Default::default()).unwrap();
            let result = interpreter.execute_cleanup(&prepared.program, &storage);
            if index == 0 {
                assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidCallPlan);
                assert!(interpreter.events().is_empty());
                assert_eq!(storage.read().unwrap(), value);
            } else {
                result.unwrap();
                assert_eq!(storage.read().unwrap(), Value::Unit);
            }
        }
    });
}

#[test]
fn cleanup_execution_retains_layouts_without_a_custom_destructor() {
    use comptime_interpretor::{Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile("trait Retire {fn drop(*self);} struct Plain {flag:bool,}");
    world(&package, |world, items| {
        let ty = nominal(&package, "Plain", vec![]);
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &ty,
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 74,
                local: 1,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                receiver.root,
                &Shape::Fields(vec![Shape::Value]),
                true,
                &mut 1000,
            )
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        assert!(prepared.graph.functions.is_empty());
        assert!(!prepared.graph.nominals.is_empty());
        let nir::Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            unreachable!()
        };
        let value = Value::Record {
            declaration,
            arguments,
            fields: vec![Value::Bool(true)],
        };
        let storage = Storage::new(value);
        let mut interpreter = Interpreter::from_iir(&prepared.graph, Default::default()).unwrap();
        interpreter
            .execute_cleanup(&prepared.program, &storage)
            .unwrap();
        assert_eq!(storage.read().unwrap(), Value::Unit);
        assert!(interpreter.events().is_empty());
    });
}

#[test]
fn cleanup_execution_uses_concrete_generic_method_instances() {
    use comptime_interpretor::{Event, Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Box where T {value:T,} impl Retire for Box<T> where T {fn drop(*self) {}}",
    );
    world(&package, |world, items| {
        let ty = nominal(
            &package,
            "Box",
            vec![nir::Term::Primitive(nir::Primitive::Bool)],
        );
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &ty,
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 75,
                local: 1,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                receiver.root,
                &Shape::Fields(vec![Shape::Value]),
                true,
                &mut 1000,
            )
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        let iir::cleanup::Step::Invoke { method, .. } = &prepared.program.steps[0] else {
            panic!("{}", prepared.program)
        };
        assert!(!method.arguments.is_empty());
        let nir::Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            unreachable!()
        };
        let storage = Storage::new(Value::Record {
            declaration,
            arguments,
            fields: vec![Value::Bool(true)],
        });
        let mut interpreter = Interpreter::from_iir(&prepared.graph, Default::default()).unwrap();
        interpreter
            .execute_cleanup(&prepared.program, &storage)
            .unwrap();
        assert_eq!(storage.read().unwrap(), Value::Unit);
        let concrete = prepared.graph.function(method).unwrap();
        assert!(concrete.function.type_parameters.is_empty());
        assert!(interpreter.events().iter().any(
            |e| matches!(e,Event::EnterFunction(declaration) if *declaration==method.declaration)
        ));
    });
}

#[test]
fn cleanup_execution_fuel_exhaustion_does_not_release_an_unfinished_owner() {
    use comptime_interpretor::{ErrorKind, Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct Endless {} impl Retire for Endless {fn drop(*self) {while true {}}}",
    );
    world(&package, |world, items| {
        let ty = nominal(&package, "Endless", vec![]);
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            &ty,
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 76,
                local: 1,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(receiver.root, &Shape::Fields(vec![]), true, &mut 1000)
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        let nir::Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            unreachable!()
        };
        let value = Value::Record {
            declaration,
            arguments,
            fields: vec![],
        };
        let storage = Storage::new(value.clone());
        let mut interpreter = Interpreter::from_iir(
            &prepared.graph,
            comptime_interpretor::Budget {
                steps: 1000,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(
            interpreter
                .execute_cleanup(&prepared.program, &storage)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(storage.read().unwrap(), value);
    });
}

fn source_executable(
    package: &nir::program::Package,
    entry: &str,
) -> Result<luna::cleanup::Executable, String> {
    world(package, |world, _| {
        let mut items = LangItems::new();
        items
            .bind_source_traits(
                package,
                &[
                    (
                        LangItem::Drop,
                        package
                            .traits
                            .iter()
                            .find(|t| t.name == "Retire")
                            .unwrap()
                            .declaration,
                    ),
                    (
                        LangItem::Copy,
                        package
                            .traits
                            .iter()
                            .find(|t| t.name == "Duplicate")
                            .unwrap()
                            .declaration,
                    ),
                ],
            )
            .unwrap();
        let entry = iir::InstanceKey {
            declaration: package
                .functions
                .iter()
                .find(|f| f.name == entry)
                .unwrap()
                .declaration,
            arguments: vec![],
        };
        luna::cleanup::prepare_source(
            package,
            world,
            &items,
            entry,
            &mut 100000,
            Budget::default(),
        )
    })
}
fn executed_names(
    package: &nir::program::Package,
    executable: &luna::cleanup::Executable,
    events: &[comptime_interpretor::Event],
) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| {
            let comptime_interpretor::Event::EnterFunction(id) = event else {
                return None;
            };
            let function = &executable
                .graph
                .functions
                .iter()
                .find(|f| f.key.declaration == *id)
                .unwrap()
                .function;
            if function.name == "drop" {
                let nir::Term::Function { parameters, .. } =
                    function.signature.callable_signature()
                else {
                    panic!("signature")
                };
                let nir::Term::Pointer(subject) = &parameters[0].ty else {
                    panic!("receiver")
                };
                let nir::Term::Nominal { declaration, .. } = subject.as_ref() else {
                    return Some("drop".into());
                };
                Some(format!(
                    "drop {}",
                    package.structs.iter().find(|s| s.declaration == *declaration).map(|s| &s.name)
                        .or_else(|| package.enums.iter().find(|e| e.declaration == *declaration).map(|e| &e.name))
                        .expect("nominal destructor receiver")
                ))
            } else {
                Some(function.name.clone())
            }
        })
        .collect()
}

#[test]
fn scheduled_cleanup_executes_source_scopes_moves_and_transferred_returns() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Left {} struct Right {} impl Retire for Left {fn drop(*self) {}} impl Retire for Right {fn drop(*self) {}} fn transfer(value:Left)->Left {return value;} fn entry() {let first=Left {};let moved=first;{let inner=Right {};};let returned=transfer(Left {});}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("SCHEDULED_CLEANUP_DUMP={executable}\nSCHEDULED_CLEANUP_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Right", "transfer", "drop Left", "drop Left"]
    );
}

#[test]
fn scheduled_cleanup_executes_returns_breaks_and_replacement_at_their_boundaries() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} impl Retire for Ticket {fn drop(*self) {}} fn early() {let held=Ticket {};return;} fn looping() {while true {let held=Ticket {};break;}} fn replacing() {var held=Ticket {};held=Ticket {};} fn self_move() {var held=Ticket {};held=held;}",
    );
    for (entry, count) in [
        ("early", 1),
        ("looping", 1),
        ("replacing", 2),
        ("self_move", 1),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        let names = executed_names(&package, &executable, interpreter.events());
        assert_eq!(
            names.iter().filter(|s| *s == "drop Ticket").count(),
            count,
            "{entry}: {names:?}"
        );
    }
}

#[test]
fn scheduled_cleanup_handles_predicate_rejection_across_call_frames() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Callee {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Callee {fn drop(*self) {}} fn reject() {let inner=Callee {};assumes false;} fn entry() {let outer=Caller {};reject();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "reject", "drop Callee", "drop Caller"]
    );
}

#[test]
fn scheduled_cleanup_discards_projected_field_and_remaining_owner() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} struct Pair {first:Ticket,second:Ticket,} impl Retire for Ticket {fn drop(*self) {}} fn entry() {Pair {first:Ticket {},second:Ticket {}}.first;}");
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(interpreter.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Unit);
    assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "drop Ticket", "drop Ticket"]);
}

#[test]
fn scheduled_cleanup_handles_continue_generics_and_field_replacement() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} struct Holder {child:Ticket,flag:bool,} impl Retire for Ticket {fn drop(*self) {}} fn transfer(value:T)->T where T {value} fn generic() {let held=transfer(Ticket {});} fn looping() {var again=true;while again {let held=Ticket {};again=false;continue;}} fn replacing()->bool {var holder=Holder {child:Ticket {},flag:true};let flag=holder.flag.ref;holder.child=Ticket {};flag.*}",
    );
    for (entry, count, result) in [
        ("generic", 1, Value::Unit),
        ("looping", 1, Value::Unit),
        ("replacing", 2, Value::Bool(true)),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            result
        );
        let names = executed_names(&package, &executable, interpreter.events());
        assert_eq!(
            names.iter().filter(|s| *s == "drop Ticket").count(),
            count,
            "{entry}: {names:?}"
        );
    }
}

#[test]
fn scheduled_cleanup_transfers_arguments_and_selects_runtime_enum_payloads() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} impl Retire for Ticket {fn drop(*self) {}} fn consume(value:Ticket) {} fn call() {consume(Ticket {});} fn entry(value:Choice) {let moved=value;}",
    );
    let call = source_executable(&package, "call").unwrap();
    let mut interpreter = Interpreter::from_iir(&call.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&call.schedule)
        .unwrap();
    interpreter.invoke_instance(&call.entry, vec![]).unwrap();
    assert_eq!(
        executed_names(&package, &call, interpreter.events()),
        ["call", "consume", "drop Ticket"]
    );
    let executable = source_executable(&package, "entry").unwrap();
    let nir::Term::Nominal {
        declaration,
        arguments,
    } = nominal(&package, "Ticket", vec![])
    else {
        unreachable!()
    };
    for tag in [0, 1] {
        let ticket = || Value::Record {
            declaration,
            arguments: arguments.clone(),
            fields: vec![],
        };
        let value = Value::Variant {
            declaration: package.enums[0].declaration,
            arguments: vec![],
            index: tag,
            fields: if tag == 0 {
                vec![]
            } else {
                vec![ticket(), ticket()]
            },
        };
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        interpreter
            .invoke_instance(&executable.entry, vec![value])
            .unwrap();
        assert_eq!(
            executed_names(&package, &executable, interpreter.events())
                .iter()
                .filter(|s| *s == "drop Ticket")
                .count(),
            if tag == 0 { 0 } else { 2 }
        );
    }
}

#[test]
fn scheduled_cleanup_stops_unwinding_after_a_destructor_rejects() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Failing {} struct Later {} impl Retire for Failing {fn drop(*self) {assumes false;}} impl Retire for Later {fn drop(*self) {}} fn nested() {let later=Later {};let failed=Failing {};} fn entry() {let caller=Later {};nested();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "nested", "drop Failing"]
    );
}

#[test]
fn scheduled_cleanup_owns_parameters_before_checking_function_preconditions() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    // External entry arguments have no source caller to prove the precondition.
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} impl Retire for Ticket {fn drop(*self) {}} fn entry(value:Ticket,flag:bool) where requires flag {}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let nir::Term::Nominal {
        declaration,
        arguments,
    } = nominal(&package, "Ticket", vec![])
    else {
        unreachable!()
    };
    let ticket = Value::Record {
        declaration,
        arguments,
        fields: vec![],
    };
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![ticket, Value::Bool(false)])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["drop Ticket"]
    );
}

#[test]
fn scheduled_discard_cleans_call_results_and_moved_locals_at_expression_end() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} impl Retire for Ticket {fn drop(*self) {}} fn make()->Ticket {Ticket {}} fn marker() {} fn entry() {let held=Ticket {};make();marker();held;marker();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("DISCARD_CLEANUP_DUMP={executable}\nDISCARD_CLEANUP_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "make",
            "drop Ticket",
            "marker",
            "drop Ticket",
            "marker"
        ]
    );
}

#[test]
fn scheduled_discard_handles_generic_transfers_tuple_and_enum_results() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} impl Retire for Ticket {fn drop(*self) {}} fn transfer(value:T)->T where T {value} fn generic() {transfer(Ticket {});} fn tuple() {(Ticket {},Ticket {});} fn variants() {Choice.empty;Choice.pair(Ticket {},Ticket {});}",
    );
    for (entry, count) in [("generic", 1), ("tuple", 2), ("variants", 2)] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        let names = executed_names(&package, &executable, interpreter.events());
        assert_eq!(
            names.iter().filter(|s| *s == "drop Ticket").count(),
            count,
            "{entry}: {names:?}"
        );
    }
}

#[test]
fn scheduled_discard_runs_only_on_executed_branches_and_loop_iterations() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} impl Retire for Ticket {fn drop(*self) {}} fn branching(flag:bool) {if flag {Ticket {};} else {Ticket {};}} fn looping() {var again=true;while again {Ticket {};again=false;continue;}} fn nested() {let flag=do {Ticket {};true};}",
    );
    for (entry, args) in [
        ("branching", vec![Value::Bool(true)]),
        ("branching", vec![Value::Bool(false)]),
        ("looping", vec![]),
        ("nested", vec![]),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        interpreter
            .invoke_instance(&executable.entry, args)
            .unwrap();
        assert_eq!(
            executed_names(&package, &executable, interpreter.events())
                .iter()
                .filter(|s| *s == "drop Ticket")
                .count(),
            1
        );
    }
}

#[test]
fn scheduled_discard_distinguishes_failed_evaluation_from_failed_destructor() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} struct Later {} impl Retire for Ticket {fn drop(*self) {assumes false;}} impl Retire for Later {fn drop(*self) {}} fn fail()->Ticket {assumes false;Ticket {}} fn evaluation() {let held=Later {};fail();} fn destructor() {let held=Later {};Ticket {};}",
    );
    for (entry, expected) in [
        ("evaluation", vec!["evaluation", "fail", "drop Later"]),
        ("destructor", vec!["destructor", "drop Ticket"]),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::ProgramContractRejected
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            expected
        );
    }
}

#[test]
fn scheduled_discard_of_a_moved_field_preserves_parent_and_sibling_cleanup() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Left {} struct Right {} struct Pair {first:Left,second:Right,} impl Retire for Left {fn drop(*self) {}} impl Retire for Right {fn drop(*self) {}} impl Retire for Pair {fn drop(*self) {}} fn entry() {let pair=Pair {first:Left {},second:Right {}};pair.first;}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Left", "drop Pair", "drop Right"]
    );
}

#[test]
fn scheduled_preparation_interleaves_completed_fields_with_inner_and_outer_locals() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Inner {} struct Outer {} struct Last {} struct Pair {first:First,last:Last,} impl Retire for First {fn drop(*self) {}} impl Retire for Inner {fn drop(*self) {}} impl Retire for Outer {fn drop(*self) {}} impl Retire for Pair {fn drop(*self) {}} fn entry() {let outer=Outer {};let pair=Pair {first:First {},last:do {let inner=Inner {};return;Last {}}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("PREPARED_CLEANUP_DUMP={executable}\nPREPARED_CLEANUP_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Inner", "drop First", "drop Outer"]
    );
}

#[test]
fn scheduled_preparation_handles_nested_records_tuples_and_variant_payloads() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} struct Last {} struct Inner {second:Second,last:Last,} struct Outer {first:First,inner:Inner,} enum Choice {pair(First,Last)} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn nested() {let outer=Outer {first:First {},inner:Inner {second:Second {},last:do {return;Last {}}}};} fn tuple() {let result=(First {},do {return;Last {}});} fn variant() {let choice=Choice.pair(First {},do {return;Last {}});}",
    );
    for (entry, expected) in [
        ("nested", vec!["nested", "drop Second", "drop First"]),
        ("tuple", vec!["tuple", "drop First"]),
        ("variant", vec!["variant", "drop First"]),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            expected
        );
    }
}

#[test]
fn scheduled_preparation_uses_normalized_argument_order_instead_of_parameter_order() {
    use comptime_interpretor::{Interpreter, Value};
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} struct Last {} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn target(first:First,second:Second,last:Last) {} fn entry() {target(First {},Second {},do {return;Last {}});}",
    );
    // Exercise the normalized argument-slot contract used by named supply.
    // Source nominal optional defaults still require unavailable P3 evidence.
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let nir::program::ExprKind::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let nir::program::ExprKind::Discard(call) = &mut statements[0].kind else {
        panic!("discard")
    };
    let nir::program::ExprKind::Call { arguments, .. } = &mut call.kind else {
        panic!("call")
    };
    arguments.swap(0, 1);
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop First", "drop Second"]
    );
}

#[test]
fn scheduled_preparation_isolates_callee_failure_from_suspended_caller_frames() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Callee {} struct Last {} struct Pair {first:First,last:Last,} impl Retire for First {fn drop(*self) {}} impl Retire for Callee {fn drop(*self) {}} fn fail(value:Callee)->Last {assumes false;Last {}} fn entry() {let pair=Pair {first:First {},last:fail(Callee {})};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "fail", "drop Callee", "drop First"]
    );
}

#[test]
fn scheduled_preparation_clears_abandoned_loop_frames_and_transfers_successful_values() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} struct Last {} struct Pair {first:Ticket,last:Last,} impl Retire for Ticket {fn drop(*self) {}} fn looping() {var again=true;while again {let pair=Pair {first:Ticket {},last:do {again=false;continue;Last {}}};}let held=Ticket {};} fn successful() {let pair=Pair {first:Ticket {},last:Last {}};}",
    );
    for (entry, count) in [("looping", 2), ("successful", 1)] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events())
                .iter()
                .filter(|name| *name == "drop Ticket")
                .count(),
            count
        );
    }
}

#[test]
fn scheduled_preparation_stops_remaining_cleanup_when_a_prepared_destructor_fails() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Earlier {} struct Failing {} struct Last {} struct Triple {first:Earlier,second:Failing,last:Last,} impl Retire for Earlier {fn drop(*self) {}} impl Retire for Failing {fn drop(*self) {assumes false;}} fn entry() {let outer=Earlier {};let triple=Triple {first:Earlier {},second:Failing {},last:do {return;Last {}}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Failing"]
    );
}

#[test]
fn scheduled_preparation_loop_transfer_preserves_enclosing_constructor_frame() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} struct Last {} struct Outer {first:First,last:Last,} struct Inner {first:Second,last:Last,} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn entry() {let result=Outer {first:First {},last:do {var again=true;while again {let inner=Inner {first:Second {},last:do {again=false;continue;Last {}}};}return;Last {}}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Second", "drop First"]
    );
}

fn normalized_cleanup_default() -> nir::program::Package {
    use nir::program::ExprKind as K;
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Input {} struct Local {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Input {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} fn target(input:Input,flag:bool,.result:bool=true)->bool {result} fn generate(flag:bool)->bool {let local=Local {};assumes flag;true} fn entry(flag:bool) {let caller=Caller {};let input=Input {};let result=target(input,flag);}",
    );
    let parameter = package
        .functions
        .iter()
        .find(|f| f.name == "target")
        .unwrap()
        .parameters[1];
    let mut body = package
        .functions
        .iter()
        .find(|f| f.name == "generate")
        .unwrap()
        .body
        .clone();
    let K::Block { statements, .. } = &mut body.kind else {
        panic!("block")
    };
    for statement in statements {
        let value = if let K::Discard(value) = &mut statement.kind {
            value.as_mut()
        } else {
            statement
        };
        if let K::CheckPredicate(condition) = &mut value.kind {
            condition.kind = K::Local(parameter);
        }
    }
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &mut statements.last_mut().unwrap().kind else {
        panic!("let")
    };
    let K::Call { arguments, .. } = &mut value.kind else {
        panic!("call")
    };
    arguments.iter_mut().find(|a| a.is_default).unwrap().value = body;
    package
}

#[test]
fn scheduled_defaults_separate_default_locals_and_caller_responsibilities() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    // Deliberately operates after default admission; source P3 is not widened.
    let package = normalized_cleanup_default();
    let executable = source_executable(&package, "entry").unwrap();
    println!("DEFAULT_CLEANUP_DUMP={executable}\nDEFAULT_CLEANUP_END");
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "drop Local", "target", "drop Input", "drop Caller"]
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "drop Local", "drop Input", "drop Caller"]
            );
        }
    }
}

#[test]
fn scheduled_defaults_allow_repeated_call_sites_without_context_collision() {
    use comptime_interpretor::{Interpreter, Value};
    use nir::program::ExprKind as K;
    let mut package = normalized_cleanup_default();
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &statements.last().unwrap().kind else {
        panic!("let")
    };
    let mut second = (**value).clone();
    // A second normalized call supplies a fresh Input and reuses the same
    // declaration-default body source. Context identity must use the call site.
    second.source.start += 10000;
    second.source.end += 10000;
    let K::Call { arguments, .. } = &mut second.kind else {
        panic!("call")
    };
    let input = package
        .structs
        .iter()
        .find(|s| s.name == "Input")
        .unwrap()
        .declaration;
    arguments[0].value.kind = K::Record {
        declaration: input,
        fields: vec![],
    };
    let statement = nir::program::Expr {
        source: second.source,
        ty: nir::Term::Primitive(nir::Primitive::Void),
        kind: K::Discard(Box::new(second)),
    };
    statements.push(statement);
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    interpreter
        .invoke_instance(&executable.entry, vec![Value::Bool(true)])
        .unwrap();
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "drop Local",
            "target",
            "drop Input",
            "drop Local",
            "target",
            "drop Input",
            "drop Caller"
        ]
    );
}

#[test]
fn scheduled_defaults_share_prepared_storage_with_callee_and_keep_failure_order() {
    use comptime_interpretor::{ErrorKind, Event, Interpreter, Value};
    use nir::program::{Expr, ExprKind as K};
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Input {flag:bool,} struct Local {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Input {fn drop(*self) {let flag=self.flag.*;assumes not flag;}} impl Retire for Local {fn drop(*self) {}} fn target(input:Input,flag:bool,.result:bool=true)->bool {input.flag} fn generate(input:Input,flag:bool)->bool {input.flag = false;let local=Local {};assumes flag;true} fn entry(flag:bool)->bool {let caller=Caller {};let input=Input {flag:true};let result=target(input,flag);result}",
    );
    let target = package
        .functions
        .iter()
        .find(|f| f.name == "target")
        .unwrap()
        .parameters
        .clone();
    let generator = package
        .functions
        .iter()
        .find(|f| f.name == "generate")
        .unwrap();
    let from = generator.parameters.clone();
    let mut body = generator.body.clone();
    fn rebind(expr: &mut Expr, from: &[nir::program::LocalId], to: &[nir::program::LocalId]) {
        match &mut expr.kind {
            K::Local(id) => {
                if let Some(index) = from.iter().position(|old| old == id) {
                    *id = to[index];
                }
            }
            K::Block { statements, result } => {
                for statement in statements {
                    rebind(statement, from, to);
                }
                if let Some(value) = result {
                    rebind(value, from, to);
                }
            }
            K::Let { value, .. }
            | K::Field { value, .. }
            | K::AddressOf(value)
            | K::Discard(value)
            | K::ReadPointer(value)
            | K::CheckPredicate(value) => rebind(value, from, to),
            K::WritePointer { pointer, value } => {
                rebind(pointer, from, to);
                rebind(value, from, to);
            }
            K::Assign { place, value } => {
                if let Some(index) = from.iter().position(|old| old == &place.root) {
                    place.root = to[index];
                }
                rebind(value, from, to);
            }
            K::Record { fields, .. } => {
                for value in fields {
                    rebind(value, from, to);
                }
            }
            K::Literal(_) => (),
            other => panic!("fixture rebind: {other:?}"),
        }
    }
    rebind(&mut body, &from, &target);
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &mut statements.last_mut().unwrap().kind else {
        panic!("let")
    };
    let K::Call { arguments, .. } = &mut value.kind else {
        panic!("call")
    };
    arguments.iter_mut().find(|a| a.is_default).unwrap().value = body;
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(
                result.unwrap(),
                Value::Bool(false),
                "callee must see the default's write"
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        let names = executed_names(&package, &executable, interpreter.events());
        assert_eq!(
            names,
            if flag {
                vec!["entry", "drop Local", "target", "drop Input", "drop Caller"]
            } else {
                vec!["entry", "drop Local", "drop Input", "drop Caller"]
            }
        );
        assert_eq!(interpreter.events().iter().filter(|event|
            matches!(event, Event::CheckPredicate {passed: true, ..})).count(),
            if flag {2} else {1}, "Drop must see the default's field write on both paths");
    }
}

#[test]
fn scheduled_defaults_nest_contexts_and_propagate_failure_in_responsibility_order() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    use nir::program::ExprKind as K;
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Input {} struct Local {} struct Inner {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Input {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} impl Retire for Inner {fn drop(*self) {}} fn target(input:Input,flag:bool,.result:bool=true)->bool {result} fn inner(input:Input,flag:bool,.result:bool=true)->bool {result} fn inner_gen(flag:bool)->bool {let local=Inner {};assumes flag;true} fn outer_gen(flag:bool)->bool {let local=Local {};let result=inner(Input {},flag);result} fn entry(flag:bool) {let caller=Caller {};let result=target(Input {},flag);}",
    );
    let target_binding = package
        .functions
        .iter()
        .find(|f| f.name == "target")
        .unwrap()
        .parameters[1];
    let inner_binding = package
        .functions
        .iter()
        .find(|f| f.name == "inner")
        .unwrap()
        .parameters[1];
    let mut inner_body = package
        .functions
        .iter()
        .find(|f| f.name == "inner_gen")
        .unwrap()
        .body
        .clone();
    let K::Block { statements, .. } = &mut inner_body.kind else {
        panic!("block")
    };
    for statement in statements {
        let expr = if let K::Discard(value) = &mut statement.kind {
            value.as_mut()
        } else {
            statement
        };
        if let K::CheckPredicate(condition) = &mut expr.kind {
            condition.kind = K::Local(inner_binding);
        }
    }
    let mut outer_body = package
        .functions
        .iter()
        .find(|f| f.name == "outer_gen")
        .unwrap()
        .body
        .clone();
    let K::Block { statements, .. } = &mut outer_body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &mut statements.last_mut().unwrap().kind else {
        panic!("let")
    };
    let K::Call { arguments, .. } = &mut value.kind else {
        panic!("call")
    };
    arguments
        .iter_mut()
        .find(|a| a.parameter == 1)
        .unwrap()
        .value
        .kind = K::Local(target_binding);
    arguments.iter_mut().find(|a| a.is_default).unwrap().value = inner_body;
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &mut statements.last_mut().unwrap().kind else {
        panic!("let")
    };
    let K::Call { arguments, .. } = &mut value.kind else {
        panic!("call")
    };
    arguments.iter_mut().find(|a| a.is_default).unwrap().value = outer_body;
    let executable = source_executable(&package, "entry").unwrap();
    assert!(
        executable
            .schedule
            .functions
            .iter()
            .any(|f| f.context.len() == 2)
    );
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            result.unwrap();
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            if flag {
                vec![
                    "entry",
                    "drop Inner",
                    "inner",
                    "drop Input",
                    "drop Local",
                    "target",
                    "drop Input",
                    "drop Caller",
                ]
            } else {
                vec![
                    "entry",
                    "drop Inner",
                    "drop Input",
                    "drop Local",
                    "drop Input",
                    "drop Caller",
                ]
            }
        );
    }
}

#[test]
fn scheduled_defaults_destructor_failure_stops_caller_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let mut package = normalized_cleanup_default();
    let failure = compile("fn fail() {assumes false;}").functions[0]
        .body
        .clone();
    let local = nominal(&package, "Local", vec![]);
    let drop_body = package
        .functions
        .iter_mut()
        .find(|function| {
            if function.name != "drop" {
                return false;
            }
            let nir::Term::Function { parameters, .. } = function.signature.callable_signature()
            else {
                return false;
            };
            parameters
                .first()
                .is_some_and(|p| p.ty == nir::Term::Pointer(Box::new(local.clone())))
        })
        .unwrap();
    drop_body.body = failure;
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap_err()
                .kind,
            ErrorKind::ProgramContractRejected
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "drop Local"]
        );
    }
}

#[test]
fn scheduled_defaults_missing_context_rejects_before_default_effects() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = normalized_cleanup_default();
    let mut executable = source_executable(&package, "entry").unwrap();
    executable
        .schedule
        .functions
        .retain(|scope| scope.context.is_empty());
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![Value::Bool(true)])
            .unwrap_err()
            .kind,
        ErrorKind::InvalidCallPlan
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry"]
    );
}

#[test]
fn scheduled_receiver_parent_writes_are_visible_to_child_destructors() {
    use comptime_interpretor::{Event, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Child {flag:bool,} struct Parent {child:Child,} impl Retire for Child {fn drop(*self) {let flag=self.flag.*;assumes not flag;}} impl Retire for Parent {fn drop(*self) {self.child.flag.* = false;}} fn entry() {let value=Parent {child:Child {flag:true}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Parent", "drop Child"]
    );
    assert_eq!(
        interpreter
            .events()
            .iter()
            .filter(|event| matches!(event, Event::CheckPredicate { passed: true, .. }))
            .count(),
        1
    );
}

#[test]
fn scheduled_receiver_is_borrowed_while_destructor_locals_are_owned() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} struct Parent {flag:bool,} impl Retire for Local {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {let local=Local {};let flag=self.flag.*;assumes flag;}} fn entry() {let value=Parent {flag:true};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Parent", "drop Local"]
    );
}

#[test]
fn scheduled_receiver_assumption_does_not_authorize_arbitrary_pointer_entries() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Cell {flag:bool,} fn entry(value:*Cell)->bool {value.flag.*}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(error.contains("provenance"), "{error}");
}

#[test]
fn scheduled_receiver_preserves_partial_owner_access_checks() {
    use comptime_interpretor::{Interpreter, Value};
    for (read, accepted) in [("self.flag.*", true), ("self.child.flag.*", false)] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Child {{flag:bool,}} struct Parent {{flag:bool,child:Child,}} impl Retire for Child {{fn drop(*self) {{}}}} impl Retire for Parent {{fn drop(*self) {{let flag={read};assumes flag;}}}} fn entry() {{let value=Parent {{flag:true,child:Child {{flag:true}}}};let child=value.child;}}"
        ));
        let executable = source_executable(&package, "entry");
        if !accepted {
            assert!(
                executable.is_err(),
                "borrowed receiver cannot revive moved fields"
            );
            continue;
        }
        let executable = executable.unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "drop Child", "drop Parent"]
        );
    }
}

#[test]
fn scheduled_receiver_rejection_cleans_destructor_locals_and_stops_outer_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} struct Parent {flag:bool,} impl Retire for Local {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {let local=Local {};let flag=self.flag.*;assumes flag;}} fn entry(flag:bool) {let outer=Local {};let value=Parent {flag:flag};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "drop Parent", "drop Local", "drop Local"]
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "drop Parent", "drop Local"]
            );
        }
    }
}

#[test]
fn scheduled_receiver_local_pointer_capture_requires_a_cleanup_footprint() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {pointer:*bool,} struct Parent {flag:bool,} impl Retire for Local {fn drop(*self) {let flag=self.pointer.*.*;assumes flag;}} impl Retire for Parent {fn drop(*self) {let local=Local {pointer:self.flag};}} fn entry() {let value=Parent {flag:true};}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(error.contains("aggregate pointer dependencies"), "{error}");
}

#[test]
fn scheduled_receiver_generic_body_owns_nested_tuple_and_variant_locals() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} enum Choice where T {none,some(T)} struct Parent where T {flag:bool,value:T,} impl Retire for Local {fn drop(*self) {}} impl Retire for Parent<T> where T {fn drop(*self) {let local=(Local {},Choice<Local>.some(Local {}));let flag=self.flag.*;assumes flag;}} fn entry() {let value=Parent<bool> {flag:true,value:false};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("BORROWED_RECEIVER_DUMP={executable}\nBORROWED_RECEIVER_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Parent", "drop Local", "drop Local"]
    );
}

#[test]
fn scheduled_memory_defaults_publish_alias_chain_contexts() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn target(p:*bool,.q:*bool=do {let saved=p;saved},.r:*bool=q)->bool {true} fn entry()->bool {let value=false;target(value.ref)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    assert_eq!(
        executable
            .schedule
            .functions
            .iter()
            .filter(|scope| !scope.context.is_empty())
            .count(),
        2
    );
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scheduled_memory_defaults_own_locals_and_restore_caller_on_rejection() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    use nir::program::{Expr, ExprKind as K};
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Local {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} fn target(p:*bool,flag:bool,.q:*bool=p,.result:bool=true)->bool {result} fn generate(p:*bool,flag:bool)->bool {let local=Local {};p.* = false;let seen=p.*;assumes flag;seen} fn entry(flag:bool)->bool {let caller=Caller {};let value=true;target(value.ref,flag)}",
    );
    let target = package
        .functions
        .iter()
        .find(|f| f.name == "target")
        .unwrap()
        .parameters
        .clone();
    let generator = package
        .functions
        .iter()
        .find(|f| f.name == "generate")
        .unwrap();
    let from = generator.parameters.clone();
    let mut body = generator.body.clone();
    fn rebind(expr: &mut Expr, from: &[nir::program::LocalId], to: &[nir::program::LocalId]) {
        match &mut expr.kind {
            K::Local(id) => {
                if let Some(index) = from.iter().position(|old| old == id) {
                    *id = to[index];
                }
            }
            K::Block { statements, result } => {
                for value in statements {
                    rebind(value, from, to);
                }
                if let Some(value) = result {
                    rebind(value, from, to);
                }
            }
            K::Let { value, .. }
            | K::ReadPointer(value)
            | K::CheckPredicate(value)
            | K::Discard(value) => rebind(value, from, to),
            K::WritePointer { pointer, value } => {
                rebind(pointer, from, to);
                rebind(value, from, to);
            }
            K::Record { fields, .. } => {
                for value in fields {
                    rebind(value, from, to);
                }
            }
            K::Literal(_) => (),
            other => panic!("fixture rebind: {other:?}"),
        }
    }
    // Read through q, itself a preceding default, to exercise composed aliases.
    rebind(&mut body, &from, &[target[2], target[1]]);
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block {
        result: Some(result),
        ..
    } = &mut entry.body.kind
    else {
        panic!("block")
    };
    let K::Call { arguments, .. } = &mut result.kind else {
        panic!("call")
    };
    arguments
        .iter_mut()
        .find(|a| a.parameter == 3)
        .unwrap()
        .value = body;
    let executable = source_executable(&package, "entry").unwrap();
    println!("MEMORY_DEFAULT_DUMP={executable}\nMEMORY_DEFAULT_END");
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Bool(false));
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "drop Local", "target", "drop Caller"]
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "drop Local", "drop Caller"]
            );
        }
    }
}

#[test]
fn scheduled_memory_defaults_recursive_calls_keep_parameter_frames_separate() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn recur(p:*bool,again:bool,.q:*bool=do {let alias=p;alias})->bool {let local=Local {};if again {recur(q,false)} else {true}} fn entry()->bool {let value=false;recur(value.ref,true)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "recur", "recur", "drop Local", "drop Local"]
    );
}

#[test]
fn scheduled_pointer_helpers_compose_drop_field_accesses() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Child {flag:bool,} struct Parent {child:Child,} fn clear(p:*bool) {p.* = false;} fn check(p:*bool) {let flag=p.*;assumes not flag;} impl Retire for Parent {fn drop(*self) {clear(self.child.flag);}} impl Retire for Child {fn drop(*self) {check(self.flag);}} fn entry() {let value=Parent {child:Child {flag:true}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Parent", "clear", "drop Child", "check"]
    );
}

#[test]
fn scheduled_pointer_helpers_allow_aliases_without_claiming_disjointness() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn update(first:*bool,second:*bool)->bool {first.* = false;second.*} fn entry()->bool {let value=true;update(value.ref,value.ref)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn scheduled_pointer_helpers_generic_defaults_return_checked_aliases() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn alias(p:*T,.q:*T=p)->*T where T {q} fn load(p:*bool)->bool {let q=alias(p);q.*} fn entry()->bool {let value=true;load(value.ref)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("POINTER_HELPER_DUMP={executable}\nPOINTER_HELPER_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "load", "alias"]
    );
}

#[test]
fn scheduled_pointer_helpers_rejection_cleans_callee_then_caller() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Callee {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Callee {fn drop(*self) {}} fn helper(p:*bool) {let local=Callee {};let flag=p.*;assumes flag;} fn entry(flag:bool) {let caller=Caller {};helper(flag.ref);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "helper", "drop Callee", "drop Caller"]
        );
    }
}

#[test]
fn scheduled_pointer_helpers_cannot_revive_partially_moved_receiver_fields() {
    use comptime_interpretor::{Interpreter, Value};
    for (field, accepted) in [("flag", true), ("child.flag", false)] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Child {{flag:bool,}} struct Parent {{flag:bool,child:Child,}} fn check(p:*Parent) {{let flag=p.{field}.*;assumes flag;}} impl Retire for Parent {{fn drop(*self) {{check(self);}}}} fn entry() {{let value=Parent {{flag:true,child:Child {{flag:true}}}};let child=value.child;}}"
        ));
        let executable = source_executable(&package, "entry");
        if !accepted {
            assert!(executable.is_err());
            continue;
        }
        let executable = executable.unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "drop Parent", "check"]
        );
    }
}

#[test]
fn scheduled_pointer_helpers_recursive_reads_keep_borrowed_parameter_identity() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn helper(p:*bool,again:bool)->bool {if again {helper(p,false)} else {p.*}} fn entry()->bool {let value=true;helper(value.ref,true)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "helper", "helper"]
    );
}

#[test]
fn scheduled_postconditions_keep_named_and_unnamed_results_until_acceptance() {
    use comptime_interpretor::{ErrorKind, Event, Interpreter, Value};
    for result in ["result:Token", "Token"] {
        let mut package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Caller {{}} struct Local {{}} struct Token {{}} impl Retire for Caller {{fn drop(*self) {{}}}} impl Retire for Local {{fn drop(*self) {{}}}} impl Retire for Token {{fn drop(*self) {{}}}} fn build(flag:bool,early:bool)->{result} where ensures true {{let local=Local {{}};if early {{return Token {{}};}}Token {{}}}} fn entry(flag:bool,early:bool) {{let caller=Caller {{}};let result=build(flag,early);}}"
        ));
        // Exercise runtime rejection after contract elaboration without changing
        // the source proof requirement for an advertised postcondition.
        let build = package
            .functions
            .iter_mut()
            .find(|f| f.name == "build")
            .unwrap();
        build.postconditions[0].kind = nir::program::ExprKind::Local(build.parameters[0]);
        let executable = source_executable(&package, "entry").unwrap();
        println!("POSTCONDITION_DUMP={executable}\nPOSTCONDITION_END");
        for flag in [true, false] {
            for early in [true, false] {
                let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
                    .unwrap()
                    .with_cleanup_schedule(&executable.schedule)
                    .unwrap();
                let value = interpreter.invoke_instance(
                    &executable.entry,
                    vec![Value::Bool(flag), Value::Bool(early)],
                );
                if flag {
                    assert_eq!(value.unwrap(), Value::Unit);
                } else {
                    assert_eq!(value.unwrap_err().kind, ErrorKind::ProgramContractRejected);
                }
                let names = executed_names(&package, &executable, interpreter.events());
                let expected = if !flag && early {
                    vec!["entry", "build", "drop Token", "drop Local", "drop Caller"]
                } else {
                    vec!["entry", "build", "drop Local", "drop Token", "drop Caller"]
                };
                assert_eq!(names, expected, "{result}, {flag}, {early}");
                assert_eq!(
                    interpreter
                        .events()
                        .iter()
                        .filter(|e| matches!(e, Event::CheckPredicate { .. }))
                        .count(),
                    1,
                    "postcheck must run once"
                );
            }
        }
    }
}

#[test]
fn scheduled_postconditions_observe_named_values_on_every_return_path() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn entry(flag:bool,other:bool)->result:bool where ensures result {if flag {return true;}if other {return true;}true}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for (flag, other) in [(true, false), (false, true), (false, false)] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(
                    &executable.entry,
                    vec![Value::Bool(flag), Value::Bool(other)]
                )
                .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn scheduled_postconditions_result_destructor_failure_stops_remaining_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    for result in ["result:Token", "Token"] {
        let mut package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Caller {{}} struct Local {{}} struct Token {{}} impl Retire for Caller {{fn drop(*self) {{}}}} impl Retire for Local {{fn drop(*self) {{}}}} impl Retire for Token {{fn drop(*self) {{assumes false;}}}} fn build(early:bool)->{result} where ensures true {{let local=Local {{}};if early {{return Token {{}};}}Token {{}}}} fn entry(early:bool) {{let caller=Caller {{}};let result=build(early);}}"
        ));
        let build = package
            .functions
            .iter_mut()
            .find(|f| f.name == "build")
            .unwrap();
        build.postconditions[0].kind =
            nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
        let executable = source_executable(&package, "entry").unwrap();
        for early in [true, false] {
            let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
                .unwrap()
                .with_cleanup_schedule(&executable.schedule)
                .unwrap();
            assert_eq!(
                interpreter
                    .invoke_instance(&executable.entry, vec![Value::Bool(early)])
                    .unwrap_err()
                    .kind,
                ErrorKind::ProgramContractRejected
            );
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                if early {
                    vec!["entry", "build", "drop Token"]
                } else {
                    vec!["entry", "build", "drop Local", "drop Token"]
                }
            );
        }
    }
}

#[test]
fn scheduled_postconditions_generic_moved_result_has_one_cleanup_owner() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Token {} impl Retire for Token {fn drop(*self) {}} fn identity(value:T,flag:bool)->result:T where T, ensures true {return value;} fn entry(flag:bool) {let result=identity(Token {},flag);}",
    );
    let identity = package
        .functions
        .iter_mut()
        .find(|f| f.name == "identity")
        .unwrap();
    identity.postconditions[0].kind = nir::program::ExprKind::Local(identity.parameters[1]);
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "identity", "drop Token"]
        );
    }
}

#[test]
fn scheduled_interface_preconditions_own_arguments_until_body_entry() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    use nir::program::ExprKind as K;
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Token {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Token {fn drop(*self) {}} trait Consume {fn consume(self,flag:bool) where requires flag;} impl Consume for Token {fn consume(self,allowed:bool) {}} fn entry(flag:bool) {let caller=Caller {};Token {}.as(Consume).consume(true);}",
    );
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let flag = entry.parameters[0];
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Discard(call) = &mut statements.last_mut().unwrap().kind else {
        panic!("discard")
    };
    let K::Call { arguments, .. } = &mut call.kind else {
        panic!("call")
    };
    arguments
        .iter_mut()
        .find(|a| a.parameter == 1)
        .unwrap()
        .value
        .kind = K::Local(flag);
    let executable = source_executable(&package, "entry").unwrap();
    println!("INTERFACE_CLEANUP_DUMP={executable}\nINTERFACE_CLEANUP_END");
    for allowed in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(allowed)]);
        if allowed {
            assert_eq!(result.unwrap(), Value::Unit);
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "consume", "drop Token", "drop Caller"]
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", "drop Token", "drop Caller"]
            );
        }
    }
}

#[test]
fn scheduled_interface_preconditions_follow_defaults_and_reverse_argument_order() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    use nir::program::ExprKind as K;
    for failing_drop in [false, true] {
        let destructor = if failing_drop { "assumes false;" } else { "" };
        let mut package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Caller {{}} struct Token {{}} struct Extra {{}} impl Retire for Caller {{fn drop(*self) {{}}}} impl Retire for Token {{fn drop(*self) {{}}}} impl Retire for Extra {{fn drop(*self) {{{destructor}}}}} trait Consume {{fn consume(self,value:Extra,.flag:bool=true) where requires flag;}} impl Consume for Token {{fn consume(self,value:Extra,allowed:bool) {{}}}} fn entry() {{let caller=Caller {{}};Token {{}}.as(Consume).consume(Extra {{}});}}"
        ));
        let entry = package
            .functions
            .iter_mut()
            .find(|f| f.name == "entry")
            .unwrap();
        let K::Block { statements, .. } = &mut entry.body.kind else {
            panic!("block")
        };
        let K::Discard(call) = &mut statements.last_mut().unwrap().kind else {
            panic!("discard")
        };
        let K::Call { arguments, .. } = &mut call.kind else {
            panic!("call")
        };
        let default = arguments.iter_mut().find(|a| a.is_default).unwrap();
        default.value.kind = K::Literal(nir::program::Literal::Bool(false));
        let executable = source_executable(&package, "entry").unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::ProgramContractRejected
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            if failing_drop {
                vec!["entry", "drop Extra"]
            } else {
                vec!["entry", "drop Extra", "drop Token", "drop Caller"]
            }
        );
    }
}

#[test]
fn scheduled_interface_preconditions_nested_construction_preserves_outer_preparation() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    use nir::program::ExprKind as K;
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Token {} struct Last {} struct Pair {first:First,last:Last,} impl Retire for First {fn drop(*self) {}} impl Retire for Token {fn drop(*self) {}} trait Consume {fn consume(self,flag:bool)->Last where requires flag;} impl Consume for Token {fn consume(self,allowed:bool)->Last {Last {}}} fn entry() {let pair=Pair {first:First {},last:Token {}.as(Consume).consume(true)};}",
    );
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &mut statements[0].kind else {
        panic!("let")
    };
    let K::Record { fields, .. } = &mut value.kind else {
        panic!("record")
    };
    let K::Call { arguments, .. } = &mut fields[1].kind else {
        panic!("call")
    };
    arguments
        .iter_mut()
        .find(|a| a.parameter == 1)
        .unwrap()
        .value
        .kind = K::Literal(nir::program::Literal::Bool(false));
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Token", "drop First"]
    );
}

#[test]
fn scheduled_residual_projection_cleans_parent_and_siblings_before_selected_local() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Selected {} struct Sibling {} struct Parent {selected:Selected,sibling:Sibling,} impl Retire for Selected {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {}} fn make()->Parent {Parent {selected:Selected {},sibling:Sibling {}}} fn entry() {let selected=make().selected;}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("RESIDUAL_DUMP={executable}\nRESIDUAL_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "make",
            "drop Parent",
            "drop Sibling",
            "drop Selected"
        ]
    );
}

#[test]
fn scheduled_residual_nested_projection_retains_each_remaining_owner_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Selected {} struct Sibling {} struct Inner {selected:Selected,sibling:Sibling,} struct Outer {inner:Inner,sibling:Sibling,} impl Retire for Selected {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} impl Retire for Inner {fn drop(*self) {}} impl Retire for Outer {fn drop(*self) {}} fn entry() {let selected=Outer {inner:Inner {selected:Selected {},sibling:Sibling {}},sibling:Sibling {}}.inner.selected;}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "drop Outer",
            "drop Sibling",
            "drop Inner",
            "drop Sibling",
            "drop Selected"
        ]
    );
}

#[test]
fn scheduled_residual_call_rejection_cleans_argument_before_remaining_temporary() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Selected {} struct Sibling {} struct Parent {selected:Selected,sibling:Sibling,} impl Retire for Caller {fn drop(*self) {}} impl Retire for Selected {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} fn reject(value:Selected) {assumes false;} fn entry() {let caller=Caller {};reject(Parent {selected:Selected {},sibling:Sibling {}}.selected);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "reject",
            "drop Selected",
            "drop Sibling",
            "drop Caller"
        ]
    );
}

#[test]
fn scheduled_residual_return_and_loop_boundaries_preserve_cleanup_order() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} struct Selected {} struct Sibling {} struct Parent {selected:Selected,sibling:Sibling,} impl Retire for Local {fn drop(*self) {}} impl Retire for Selected {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} fn make()->Selected {let local=Local {};return Parent {selected:Selected {},sibling:Sibling {}}.selected;} fn entry() {let result=make();} fn looping() {var again=true;while again {let selected=Parent {selected:Selected {},sibling:Sibling {}}.selected;again=false;continue;}}",
    );
    for (entry, expected) in [
        (
            "entry",
            vec![
                "entry",
                "make",
                "drop Sibling",
                "drop Local",
                "drop Selected",
            ],
        ),
        ("looping", vec!["looping", "drop Sibling", "drop Selected"]),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            expected
        );
    }
}

#[test]
fn scheduled_residual_default_and_function_result_preserve_their_scope_order() {
    use comptime_interpretor::{Interpreter, Value};
    use nir::program::ExprKind as K;
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Input {} struct Local {} struct Sibling {} struct Parent {flag:bool,sibling:Sibling,} impl Retire for Input {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} fn target(input:Input,.flag:bool=true)->bool {flag} fn generate()->bool {let local=Local {};Parent {flag:true,sibling:Sibling {}}.flag} fn entry()->bool {target(Input {})}",
    );
    let body = package
        .functions
        .iter()
        .find(|f| f.name == "generate")
        .unwrap()
        .body
        .clone();
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block {
        result: Some(result),
        ..
    } = &mut entry.body.kind
    else {
        panic!("block")
    };
    let K::Call { arguments, .. } = &mut result.kind else {
        panic!("call")
    };
    arguments.iter_mut().find(|a| a.is_default).unwrap().value = body;
    for (entry, expected) in [
        (
            "entry",
            vec![
                "entry",
                "drop Local",
                "drop Sibling",
                "target",
                "drop Input",
            ],
        ),
        ("generate", vec!["generate", "drop Sibling", "drop Local"]),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            expected
        );
    }
}

#[test]
fn scheduled_residual_destructor_failure_stops_sibling_and_selected_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Selected {} struct Sibling {} struct Parent {selected:Selected,sibling:Sibling,} impl Retire for Selected {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {assumes false;}} fn entry() {let selected=Parent {selected:Selected {},sibling:Sibling {}}.selected;}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Parent"]
    );
}

#[test]
fn scheduled_pattern_subject_destructuring_transfers_selected_fields() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Selected {} struct Sibling {} struct Parent {selected:Selected,sibling:Sibling,} impl Retire for Selected {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {}} fn entry() {let {selected}=Parent {selected:Selected {},sibling:Sibling {}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Parent", "drop Sibling", "drop Selected"]
    );
}

#[test]
fn scheduled_pattern_subject_tuple_wildcards_and_transfers_clean_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Selected {} struct Sibling {} impl Retire for Selected {fn drop(*self) {}} impl Retire for Sibling {fn drop(*self) {}} fn entry() {let (selected,_)= (Selected {},Sibling {});}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Sibling", "drop Selected"]
    );
}

#[test]
fn scheduled_pattern_subject_failed_let_else_cleans_original_variant() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Item {} enum Choice {none,some(Item)} impl Retire for Caller {fn drop(*self) {}} impl Retire for Item {fn drop(*self) {}} fn entry() {let caller=Caller {};let .none=Choice.some(Item {}) else {return;};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item", "drop Caller"]
    );
}

#[test]
fn scheduled_pattern_subject_guard_rollback_does_not_transfer_candidates() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} enum Choice {none,pair(First,Second)} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn entry(flag:bool) {let .pair(first,second) if flag=Choice.pair(First {},Second {}) else {return;};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("PATTERN_SUBJECT_DUMP={executable}\nPATTERN_SUBJECT_END");
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "drop Second", "drop First"]
        );
    }
}

#[test]
fn scheduled_pattern_subject_parent_drop_cannot_read_transferred_fields() {
    let package = compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Selected {flag:bool,} struct Parent {selected:Selected,} impl Retire for Parent {fn drop(*self) {let flag=self.selected.flag.*;}} fn entry() {let {selected}=Parent {selected:Selected {flag:true}};}");
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn scheduled_match_presence_cleans_only_the_selected_arm() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Left {} struct Right {} enum Choice {left(Left),right(Right)} impl Retire for Left {fn drop(*self) {}} impl Retire for Right {fn drop(*self) {}} fn entry(flag:bool) {let value=if flag {Choice.left(Left {})} else {Choice.right(Right {})};value match {.left(item)=>(),.right(item)=>()};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("MATCH_PRESENCE_DUMP={executable}\nMATCH_PRESENCE_END");
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            if flag {
                vec!["entry", "drop Left"]
            } else {
                vec!["entry", "drop Right"]
            }
        );
    }
}

#[test]
fn scheduled_match_presence_guard_fallback_preserves_transfer_order() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} enum Choice {none,pair(First,Second)} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn entry(flag:bool) {Choice.pair(First {},Second {}) match {.pair(first,_) if flag=>{},_=>{}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            if flag {
                vec!["entry", "drop First", "drop Second"]
            } else {
                vec!["entry", "drop Second", "drop First"]
            }
        );
    }
}

#[test]
fn scheduled_match_presence_early_return_transfers_binding_and_cleans_subject() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} enum Choice {none,pair(First,Second)} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn take(value:Choice)->First {value match {.pair(first,_)=>{return first;},.none=>First {}}} fn entry() {let first=take(Choice.pair(First {},Second {}));}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "take", "drop Second", "drop First"]
    );
}

#[test]
fn scheduled_match_presence_repeated_arms_clear_each_iteration() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} enum Choice {none,some(Item)} impl Retire for Item {fn drop(*self) {}} fn entry() {var left:.u32'builtin=2;while left > 0 {Choice.some(Item {}) match {.some(_)=>{},.none=>{}};left=left - 1;}}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item", "drop Item"]
    );
}

#[test]
fn scheduled_match_presence_failed_destructor_stops_remaining_branch_resources() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct First {} struct Second {} enum Choice {none,pair(First,Second)} impl Retire for Caller {fn drop(*self) {}} impl Retire for First {fn drop(*self) {assumes false;}} impl Retire for Second {fn drop(*self) {}} fn entry(flag:bool) {let caller=Caller {};Choice.pair(First {},Second {}) match {.pair(first,_) if flag=>{},_=>{}};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap_err()
                .kind,
            ErrorKind::ProgramContractRejected
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            if flag {
                vec!["entry", "drop First"]
            } else {
                vec!["entry", "drop Second", "drop First"]
            }
        );
    }
}

#[test]
fn cleanup_execution_closure_environment_uses_capture_order() {
    use comptime_interpretor::{Event, Interpreter, Storage, Value};
    use elaboration::verification::{
        PlaceId,
        ownership::{Place, Shape, State},
    };
    let package = compile(
        "trait Retire {fn drop(*self);} struct First {} struct Second {} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} typealias Captured=(do {let first=First {};let second=Second {};|| (first,second)})'type; fn make()->Captured {let first=First {};let second=Second {};|| (first,second)}",
    );
    let function = package.functions.iter().find(|f| f.name == "make").unwrap();
    let nir::Term::Function { result, .. } = function.signature.callable_signature() else {
        panic!("signature")
    };
    let value = Interpreter::new(&package, Default::default())
        .unwrap()
        .invoke(function.declaration, vec![])
        .unwrap();
    world(&package, |world, items| {
        let acquired = luna::protocols::acquire_cleanup(
            &package,
            world,
            items,
            result,
            Budget::default(),
            &mut 100000,
        )
        .unwrap();
        let receiver = Place {
            root: PlaceId {
                owner: 70,
                local: 0,
            },
            fields: vec![],
        };
        let mut state = State::default();
        state
            .declare(
                receiver.root,
                &Shape::Fields(vec![Shape::Fields(vec![]), Shape::Fields(vec![])]),
                true,
                &mut 10000,
            )
            .unwrap();
        let prepared = luna::cleanup::prepare(
            &package,
            world,
            &acquired,
            &state,
            &receiver,
            &[],
            &mut 100000,
            Budget::default(),
        )
        .unwrap();
        let storage = Storage::new(value);
        let mut interpreter = Interpreter::from_iir(&prepared.graph, Default::default()).unwrap();
        interpreter
            .execute_cleanup(&prepared.program, &storage)
            .unwrap();
        assert_eq!(storage.read().unwrap(), Value::Unit);
        let calls: Vec<_> = interpreter
            .events()
            .iter()
            .filter_map(|event| {
                if let Event::EnterFunction(id) = event {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        let drops: Vec<_> = package
            .functions
            .iter()
            .filter(|f| f.name == "drop")
            .map(|f| f.declaration)
            .collect();
        assert_eq!(calls, vec![drops[1], drops[0]]);
    });
}

#[test]
fn scheduled_closure_construction_drops_uncalled_capture_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn entry() {let first=First {};let second=Second {};let closure=|| (first,second);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("CLOSURE_STORAGE_DUMP={executable}\nCLOSURE_STORAGE_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Second", "drop First"]
    );
}

#[test]
fn scheduled_closure_construction_nested_environments_and_discards() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Last {} impl Retire for First {fn drop(*self) {}} impl Retire for Last {fn drop(*self) {}} fn entry() {let first=First {};let inner=|| first;let last=Last {};let outer=|| (inner,last);} fn discarded() {let first=First {};|| first;}",
    );
    for (entry, expected) in [
        ("entry", vec!["entry", "drop Last", "drop First"]),
        ("discarded", vec!["discarded", "drop First"]),
    ] {
        let executable = source_executable(&package, entry).unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            expected
        );
    }
}

#[test]
fn scheduled_closure_construction_failed_capture_cleans_prepared_prefix() {
    use comptime_interpretor::{ErrorKind, Event, Interpreter};
    use nir::program::ExprKind as K;
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct First {} struct Second {} impl Retire for Caller {fn drop(*self) {}} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn fail()->Second {assumes false;Second {}} fn generator()->Second {fail()} fn entry() {let caller=Caller {};let first=First {};let second=Second {};let closure=|| (first,second);}",
    );
    let generator = &package
        .functions
        .iter()
        .find(|f| f.name == "generator")
        .unwrap()
        .body;
    let K::Block {
        result: Some(value),
        ..
    } = &generator.kind
    else {
        panic!("block")
    };
    let value = value.clone();
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Let { value: closure, .. } = &mut statements.last_mut().unwrap().kind else {
        panic!("let")
    };
    let K::Closure { captures, .. } = &mut closure.kind else {
        panic!("closure")
    };
    captures[1].value = Some(value);
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "fail", "drop First", "drop Second", "drop Caller"]
    );
    assert!(
        !interpreter
            .events()
            .iter()
            .any(|e| matches!(e, Event::CreateClosure { .. }))
    );
}

#[test]
fn scheduled_closure_invocation_transfers_capture_without_double_drop() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let closure=|| item;let taken=closure();}");
    let executable = source_executable(&package,"entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(interpreter.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Unit);
    assert_eq!(executed_names(&package,&executable,interpreter.events()),["entry","drop Item"]);
}

#[test]
fn scheduled_closure_construction_capture_destructor_reads_and_writes_its_fields() {
    use comptime_interpretor::{Event, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {flag:bool,} struct Second {flag:bool,} impl Retire for First {fn drop(*self) {let flag=self.flag.*;assumes flag;}} impl Retire for Second {fn drop(*self) {self.flag.* = false;let flag=self.flag.*;assumes not flag;}} fn entry() {let first=First {flag:true};let second=Second {flag:true};let closure=|| (first,second);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Second", "drop First"]
    );
    assert_eq!(
        interpreter
            .events()
            .iter()
            .filter(|e| matches!(e, Event::CheckPredicate { passed: true, .. }))
            .count(),
        2
    );
}

#[test]
fn scheduled_closure_invocation_repeated_mutation_uses_shared_capture_storage() {
    use comptime_interpretor::{Interpreter, Value};
    use nir::program::{Expr, ExprKind as K};
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn generate(flag:bool)->bool {flag=not flag;flag} fn entry()->bool {var flag=true;let toggle=|| flag;let first=toggle();let second=toggle();not first and second}",
    );
    let generator = package
        .functions
        .iter()
        .find(|f| f.name == "generate")
        .unwrap();
    let from = generator.parameters[0];
    let mut body = generator.body.clone();
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &mut statements[1].kind else {
        panic!("let")
    };
    let K::Closure {
        captures,
        body: target,
        ..
    } = &mut value.kind
    else {
        panic!("closure")
    };
    let to = captures[0].binding;
    fn rebind(expr: &mut Expr, from: nir::program::LocalId, to: nir::program::LocalId) {
        match &mut expr.kind {
            K::Local(id) => {
                if *id == from {
                    *id = to;
                }
            }
            K::Assign { place, value } => {
                if place.root == from {
                    place.root = to;
                }
                rebind(value, from, to);
            }
            K::Not(value) | K::Discard(value) => rebind(value, from, to),
            K::Block { statements, result } => {
                for value in statements {
                    rebind(value, from, to);
                }
                if let Some(value) = result {
                    rebind(value, from, to);
                }
            }
            other => panic!("fixture rebind: {other:?}"),
        }
    }
    rebind(&mut body, from, to);
    *target = std::sync::Arc::new(body);
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scheduled_closure_invocation_partial_capture_cleanup_preserves_remaining_fields() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {flag:bool,} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn take(value:First) {} fn entry() {let first=First {};let second=Second {flag:true};let closure=|| {take(first);let flag=second.flag;};closure();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "take", "drop First", "drop Second"]
    );
}

#[test]
fn scheduled_closure_invocation_nested_contexts_clean_invocation_locals() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} struct Local {} impl Retire for Item {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} fn entry() {let item=Item {};let outer=|| {let local=Local {};let inner=|| item;inner()};let result=outer();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("CLOSURE_INVOCATION_DUMP={executable}\nCLOSURE_INVOCATION_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Local", "drop Item"]
    );
}

#[test]
fn scheduled_closure_invocation_failure_cleans_locals_and_remaining_environment() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct First {} struct Second {flag:bool,} struct Local {} impl Retire for Caller {fn drop(*self) {}} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} fn take(value:First) {} fn entry(flag:bool) {let caller=Caller {};let first=First {};let second=Second {flag:flag};let closure=|| {take(first);let local=Local {};let flag=second.flag;assumes flag;};closure();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            [
                "entry",
                "take",
                "drop First",
                "drop Local",
                "drop Second",
                "drop Caller"
            ]
        );
    }
}

#[test]
fn scheduled_closure_invocation_captureless_calls_clean_arguments() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let closure=|item:Item| {};closure(Item {});}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item"]
    );
}

#[test]
fn scheduled_closure_invocation_cannot_reuse_consumed_capture() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let closure=|| item;let first=closure();let second=closure();}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(
        error.contains("unavailable") || error.contains("move"),
        "{error}"
    );
}

#[test]
fn scheduled_closure_invocation_failed_local_drop_stops_caller_environment_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} struct Local {} impl Retire for Item {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {assumes false;}} fn entry() {let item=Item {flag:true};let closure=|| {let local=Local {};let flag=item.flag;};closure();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Local"]
    );
}
#[test]
fn scheduled_closure_invocation_keeps_return_contracts_in_their_own_scope() {
    use comptime_interpretor::{Interpreter, Value};
    for body in ["false", "return false;"] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Local {{}} impl Retire for Local {{fn drop(*self) {{}}}} fn entry()->result:bool where ensures result {{let closure=|| {{let local=Local {{}};{body}}};let ignored=closure();true}}"
        ));
        let executable = source_executable(&package, "entry").unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "drop Local"]
        );
    }
}

#[test]
fn scheduled_closure_invocation_reuses_context_in_a_loop() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn entry() {let closure=|| {let local=Local {};};var again=true;var last=false;while again {closure();again=not last;last=true;}closure();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Local", "drop Local", "drop Local"]
    );
}

#[test]
fn scheduled_closure_invocation_missing_context_rejects_before_body_effects() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn entry() {let closure=|| {let local=Local {};};closure();}",
    );
    let mut executable = source_executable(&package, "entry").unwrap();
    executable
        .schedule
        .functions
        .retain(|scope| scope.context.is_empty());
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::InvalidCallPlan
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry"]
    );
}

#[test]
fn scheduled_captured_callable_transfers_nested_capture_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let inner=|| item;let outer=|| inner();let result=outer();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item"]
    );
}

#[test]
fn scheduled_captured_callable_rejects_nested_capture_reuse() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let inner=|| item;let outer=|| inner();let first=outer();let second=outer();}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(
        error.contains("unavailable") || error.contains("move"),
        "{error}"
    );
}

#[test]
fn scheduled_captured_callable_repeated_reads_preserve_nested_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} fn entry()->bool {let item=Item {flag:true};let inner=|| item.flag;let middle=|| inner();let outer=|| middle();let first=outer();let second=outer();first and second}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item"]
    );
}

#[test]
fn scheduled_captured_callable_failure_preserves_remaining_nested_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {flag:bool,} struct Local {} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} fn take(value:First) {} fn entry(flag:bool) {let first=First {};let second=Second {flag:flag};let inner=|| {take(first);let flag=second.flag;assumes flag;};let outer=|| {let local=Local {};inner();};outer();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "take", "drop First", "drop Local", "drop Second"]
        );
    }
}

#[test]
fn scheduled_captured_callable_move_preserves_definition_and_nested_provenance() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let inner=|| item;let outer=|| inner();let moved=outer;let result=moved();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item"]
    );
}

#[test]
fn scheduled_captured_callable_without_invocation_only_cleans_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let inner=|| {assumes false;item};let outer=|| inner();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    assert!(
        executable
            .schedule
            .functions
            .iter()
            .all(|scope| scope.context.is_empty())
    );
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item"]
    );
}

#[test]
fn scheduled_captured_callable_replacement_updates_later_invocations() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first()->bool {true} fn second()->bool {false} fn entry()->bool {var inner=first;let outer=|| {let previous=inner();inner=second;previous};let before=outer();let after=outer();before and not after}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(interpreter.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "first", "second"]);
}

#[test]
fn scheduled_captured_callable_initializer_resamples_later_provenance() {
    use nir::program::{Expr, ExprKind as K, Place};
    use nir::{Primitive, Term};
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn changed()->bool {false} fn entry()->bool {var inner=|| true;let trigger=true;let outer=|| {let ignored=trigger;inner()};outer()}",
    );
    let changed = package.functions.iter().find(|f| f.name == "changed").unwrap().declaration;
    let entry = package
        .functions
        .iter_mut()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &mut entry.body.kind else {
        panic!("entry block")
    };
    let K::Let {
        binding: inner,
        value,
    } = &statements[0].kind
    else {
        panic!("inner binding")
    };
    let binding = *inner;
    let ty = value.ty.clone();
    let K::Let { value, .. } = &mut statements[2].kind else {
        panic!("outer binding")
    };
    let source = value.source;
    let K::Closure { captures, .. } = &mut value.kind else {
        panic!("outer closure")
    };
    assert_eq!(captures[1].binding, binding);
    let first = &mut captures[0];
    first.value = Some(Box::new(Expr {
        source,
        ty: first.ty.clone(),
        kind: K::Block {
            statements: vec![Expr {
                source,
                ty: Term::Primitive(Primitive::Void),
                kind: K::Assign {
                    place: Place {
                        root: binding,
                        fields: vec![],
                    },
                    value: Box::new(Expr {
                        source,
                        ty,
                        kind: K::Function(changed),
                    }),
                },
            }],
            result: Some(Box::new(Expr {
                source,
                ty: first.ty.clone(),
                kind: K::Local(first.binding),
            })),
        },
    }));
    let executable = source_executable(&package, "entry").unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),comptime_interpretor::Value::Bool(false));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","changed"]);
}

#[test]
fn scheduled_closure_parameter_calls_known_callback() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn entry()->bool {let apply=|callback:fn()->bool| callback();apply(|| {let local=Local {};true})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Local"]
    );
}

#[test]
fn scheduled_closure_parameter_transfers_callback_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let inner=|| item;let apply=|callback:inner'type| callback();let result=apply(inner);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item"]
    );
}

#[test]
fn scheduled_closure_parameter_rejects_callback_capture_reuse() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {};let inner=|| item;let apply=|callback:inner'type| {let first=callback();let second=callback();};apply(inner);}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(
        error.contains("unavailable") || error.contains("move"),
        "{error}"
    );
}

#[test]
fn scheduled_closure_parameter_uses_call_specific_callback_definition() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn entry()->bool {let apply=|callback:fn()->bool| callback();let first=apply(|| {let item=First {};false});let second=apply(|| {let item=Second {};true});not first and second}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("CALLBACK_PARAMETER_DUMP={executable}\nCALLBACK_PARAMETER_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop First", "drop Second"]
    );
}

#[test]
fn scheduled_closure_parameter_failure_cleans_remaining_environment_and_locals() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {flag:bool,} struct Local {} struct Caller {} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} fn take(value:First) {} fn entry(flag:bool) {let caller=Caller {};let first=First {};let second=Second {flag:flag};let inner=|| {take(first);let flag=second.flag;assumes flag;};let apply=|callback:inner'type| {let local=Local {};callback();};apply(inner);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            [
                "entry",
                "take",
                "drop First",
                "drop Local",
                "drop Second",
                "drop Caller"
            ]
        );
    }
}

#[test]
fn scheduled_closure_parameter_failed_later_argument_cleans_prepared_callback() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} struct Caller {} impl Retire for Item {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} fn fail()->bool {assumes false;true} fn entry() {let caller=Caller {};let item=Item {flag:true};let inner=|| item.flag;let apply=|callback:inner'type,flag:bool| callback();let ignored=apply(inner,fail());}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "fail", "drop Item", "drop Caller"]
    );
}

#[test]
fn scheduled_closure_parameter_freezes_provenance_at_argument_evaluation() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn entry()->bool {let apply=|callback:fn()->bool,flag:bool| callback();var candidate=|| true;apply(candidate,do {candidate=|| false;true})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scheduled_closure_parameter_reports_construction_obligations_separately() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn entry()->bool {let apply=|callback:fn()->bool| callback();apply(|| true)}",
    );
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    let report = moves::analyze_in_package(
        &package,
        entry,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Function { .. }) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        100000,
    )
    .unwrap();
    assert!(report.deferred_calls.is_empty());
    assert!(
        report
            .closures
            .iter()
            .any(|closure| !closure.invoked && closure.invocation.deferred_calls.len() == 1)
    );
    fn checked(report: &moves::Report) {
        assert!(report.deferred_calls.is_empty());
        for closure in report.closures.iter().filter(|closure| closure.invoked) {
            checked(&closure.invocation);
        }
    }
    checked(&report);
    println!("CALLBACK_REPORT_DUMP={report}\nCALLBACK_REPORT_END");
}

#[test]
fn callable_input_cleanup_uses_remaining_callback_environment() {
    use elaboration::verification::{
        drop::CleanupStep,
        moves::{CallableParameter, CleanupAction},
    };
    use nir::program::ExprKind as K;
    for consumed in [false, true] {
        let body = if consumed {
            "take(item);"
        } else {
            "let flag=item.flag;"
        };
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Item {{flag:bool,}} impl Retire for Item {{fn drop(*self) {{}}}} fn take(item:Item) {{}} fn apply(callback:%{{item:Item}} fn()) {{callback();}} fn entry() {{let item=Item {{flag:true}};let callback=|| {{{body}}};}}"
        ));
        let entry = package
            .functions
            .iter()
            .find(|f| f.name == "entry")
            .unwrap();
        let K::Block { statements, .. } = &entry.body.kind else {
            panic!("entry")
        };
        let K::Let { value, .. } = &statements.last().unwrap().kind else {
            panic!("callback")
        };
        let inputs = [CallableParameter { choices: vec![], enum_captures: Default::default(), fields: vec![], captures: Default::default(),
            alternatives: vec![],
            parameter: 0,
            definition: value.as_ref().clone(),
        }];
        let report = world(&package, |world, items| {
            let mut items = items.clone();
            items
                .bind_source_traits(
                    &package,
                    &[(
                        LangItem::Copy,
                        package
                            .traits
                            .iter()
                            .find(|t| t.name == "Duplicate")
                            .unwrap()
                            .declaration,
                    )],
                )
                .unwrap();
            luna::protocols::analyze_cleanup_with_inputs(
                &package,
                world,
                &items,
                package
                    .functions
                    .iter()
                    .find(|f| f.name == "apply")
                    .unwrap(),
                100000,
                Budget::default(),
                &[],
                &inputs,
            )
            .unwrap()
        });
        assert_eq!(report.callable_parameters, inputs);
        let invokes = report
            .cleanup_sites
            .iter()
            .flat_map(|site| &site.actions)
            .flat_map(|action| {
                let CleanupAction::Planned { plan, .. } = action else {
                    panic!("unplanned cleanup")
                };
                plan.steps.iter()
            })
            .filter(|step| matches!(step, CleanupStep::Invoke { .. }))
            .count();
        assert_eq!(invokes, usize::from(!consumed), "{report}");
    }
}

#[test]
fn scheduled_named_callback_selects_distinct_same_typed_bodies() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct First {} struct Second {} impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {let first=apply(|| {let value=First {};false});let second=apply(|| {let value=Second {};true});not first and second}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("NAMED_CALLBACK_DUMP={executable}\nNAMED_CALLBACK_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "drop First", "apply", "drop Second"]
    );
}

#[test]
fn scheduled_named_callback_transfers_owned_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn take(item:Item) {} fn apply(callback:%{item:Item} fn()) {callback();} fn entry() {let item=Item {};let callback=|| take(item);apply(callback);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "take", "drop Item"]
    );
}

#[test]
fn scheduled_named_callback_forwards_through_named_and_closure_scopes() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn apply(callback:fn()->bool)->bool {callback()} fn forward(callback:fn()->bool)->bool {let invoke=|| apply(callback);invoke()} fn entry()->bool {forward(|| {let local=Local {};true})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "forward", "apply", "drop Local"]
    );
}

#[test]
fn scheduled_named_callback_rejects_repeated_owned_capture_transfer() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn take(item:Item) {} fn apply(callback:%{item:Item} fn()) {callback();callback();} fn entry() {let item=Item {};let callback=|| take(item);apply(callback);}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(
        error.contains("unavailable") || error.contains("move"),
        "{error}"
    );
}

#[test]
fn scheduled_named_callback_failure_cleans_parameter_environment_before_caller() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} struct Local {} struct Caller {} impl Retire for Item {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} fn apply(callback:%{item:Item} fn()) {let local=Local {};callback();} fn entry(flag:bool) {let caller=Caller {};let item=Item {flag:flag};let callback=|| {let flag=item.flag;assumes flag;};apply(callback);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "apply", "drop Local", "drop Item", "drop Caller"]
        );
    }
}

#[test]
fn scheduled_named_callback_default_argument_gets_concrete_context() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn apply(.callback:fn()->bool=|| {let local=Local {};true})->bool {callback()} fn entry()->bool {apply()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "drop Local"]
    );
}

#[test]
fn scheduled_named_callback_contract_failure_uses_callee_scope() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    use nir::program::{ExprKind as K, Literal};
    for precondition in [true, false] {
        let mut package = compile(
            "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} struct Caller {} impl Retire for Item {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} fn apply(callback:%{item:Item} fn()->Item)->result:Item where requires true, ensures true {callback()} fn entry() {let caller=Caller {};let item=Item {};let callback=|| item;let result=apply(callback);}",
        );
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.name == "apply")
            .unwrap();
        if precondition {
            function
                .preconditions
                .push(function.postconditions[0].clone());
        }
        let condition = if precondition {
            &mut function.preconditions[0]
        } else {
            &mut function.postconditions[0]
        };
        condition.kind = K::Literal(Literal::Bool(false));
        let executable = source_executable(&package, "entry").unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap_err()
                .kind,
            ErrorKind::ProgramContractRejected
        );
        let expected = if precondition {
            vec!["entry", "drop Item", "drop Caller"]
        } else {
            vec!["entry", "apply", "drop Item", "drop Caller"]
        };
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            expected
        );
    }
}

#[test]
fn scheduled_named_callback_missing_context_rejects_before_callee_effects() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {apply(|| true)}",
    );
    let mut executable = source_executable(&package, "entry").unwrap();
    executable
        .schedule
        .functions
        .retain(|scope| scope.context.is_empty());
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::InvalidCallPlan
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry"]
    );
}

#[test]
fn scheduled_named_callback_generic_instances_and_explicit_return() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn apply(callback:fn()->T)->T where T {return callback();} fn entry()->bool {let flag=apply(|| true);let letter=apply(|| 'x');flag and letter == 'x'}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "apply"]
    );
}

#[test]
fn scheduled_named_callback_loop_reuses_one_static_call_context() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn apply(callback:fn()) {callback();} fn entry() {var again=true;var last=false;while again {apply(|| {let local=Local {};});again=not last;last=true;}} ",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "drop Local", "apply", "drop Local"]
    );
}

#[test]
fn scheduled_named_callback_later_argument_cannot_change_prepared_definition() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn apply(callback:fn()->bool,flag:bool)->bool {callback()} fn entry()->bool {var callback=|| true;apply(callback,do {callback=|| false;true})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scheduled_named_callback_preserves_captured_callable_provenance() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} typealias Reader = %{item:Item} fn()->bool; typealias Outer = %{inner:Reader} fn()->bool; fn apply(callback:Outer)->bool {callback()} fn entry()->bool {let item=Item {flag:true};let inner=|| item.flag;let outer=|| inner();apply(outer)}");
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(interpreter.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "apply", "drop Item"]);
}

#[test]
fn scheduled_named_value_callback_preserves_function_body_cleanup() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn produce()->bool {let local=Local {};true} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {apply(produce)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "produce", "drop Local"]
    );
}

#[test]
fn scheduled_named_value_indirect_higher_order_call_keeps_context() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} impl Retire for Local {fn drop(*self) {}} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {let invoke=apply;invoke(|| {let local=Local {};true})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("NAMED_VALUE_DUMP={executable}\nNAMED_VALUE_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "drop Local"]
    );
}

#[test]
fn scheduled_named_value_generic_callback_uses_concrete_instance() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn identity(value:T)->T where T {value} fn apply(callback:fn(bool)->bool)->bool {callback(true)} fn entry()->bool {apply(identity<bool>)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "identity"]
    );
}

#[test]
fn scheduled_named_value_captured_in_closure_forwards_as_callback() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn produce()->bool {true} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {let callback=produce;let closure=|| apply(callback);closure()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "produce"]
    );
}

#[test]
fn scheduled_named_value_argument_side_effect_cannot_replace_selected_callee() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first(value:bool)->bool {true} fn second(value:bool)->bool {false} fn entry()->bool {var selected=first;selected(do {selected=second;true})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "first"]
    );
}

#[test]
fn scheduled_named_value_failure_cleans_callee_then_caller() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} struct Caller {} impl Retire for Local {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} fn fail()->bool {let local=Local {};assumes false;true} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {let caller=Caller {};apply(fail)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "apply", "fail", "drop Local", "drop Caller"]
    );
}

#[test]
fn scheduled_named_value_indirect_memory_helper_retains_checked_footprint() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn flip(value:*bool) {value.* = not value.*;} fn entry()->bool {let value=true;let helper=flip;helper(value.ref);not value}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "flip"]
    );
}

#[test]
fn scheduled_callable_result_block_keeps_local_closure_after_scope_exit() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} struct Local {} impl Retire for Item {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} fn entry() {let callback=do {let local=Local {};let item=Item {};let closure=|| item;closure};let result=callback();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Local", "drop Item"]
    );
}

#[test]
fn scheduled_callable_result_block_argument_preserves_single_evaluation() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn prepare() {} fn selected()->bool {true} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {apply(do {prepare();let callback=selected;callback})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "prepare", "apply", "selected"]
    );
}

#[test]
fn scheduled_callable_result_local_factory_transfers_environment() {
    use comptime_interpretor::{Interpreter, Value};
    for body in ["|| item", "return || item;"] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Item {{}} struct Local {{}} impl Retire for Item {{fn drop(*self) {{}}}} impl Retire for Local {{fn drop(*self) {{}}}} fn entry() {{let factory=|| {{let local=Local {{}};let item=Item {{}};{body}}};let callback=factory();let result=callback();}}"
        ));
        let executable = source_executable(&package, "entry").unwrap();
        println!("CALLABLE_RESULT_DUMP={executable}\nCALLABLE_RESULT_END");
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "drop Local", "drop Item"]
        );
    }
}

#[test]
fn scheduled_callable_result_factory_result_moves_into_named_callback_parameter() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn take(callback:%{item:Item} fn()->Item)->Item {callback()} fn entry() {let factory=|| {let item=Item {};|| item};let result=take(factory());}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "take", "drop Item"]
    );
}

#[test]
fn scheduled_callable_result_merges_same_named_target_across_successful_exits() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn selected()->bool {true} fn apply(callback:fn()->bool)->bool {callback()} fn entry(flag:bool)->bool {let factory=|early:bool| {if early {return selected;}selected};let result=apply(factory(flag));let joined=if flag {selected} else {selected};let right=joined();result and right}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "apply", "selected", "selected"]
        );
    }
}

#[test]
fn scheduled_callable_result_returned_environment_cannot_be_consumed_twice() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {let factory=|| {let item=Item {};|| item};let callback=factory();let first=callback();let second=callback();}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(
        error.contains("move") || error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn scheduled_callable_result_failed_factory_does_not_publish_returned_environment() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Local {} struct Item {} struct Caller {} impl Retire for Local {fn drop(*self) {}} impl Retire for Item {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} fn entry(flag:bool) {let caller=Caller {};let factory=|flag:bool| {let local=Local {};assumes flag;let item=Item {};|| item};let callback=factory(flag);let result=callback();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        let expected = if flag {
            vec!["entry", "drop Local", "drop Item", "drop Caller"]
        } else {
            vec!["entry", "drop Local", "drop Caller"]
        };
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            expected
        );
    }
}

#[test]
fn scheduled_named_return_factory_delivers_owned_closure() {
    use comptime_interpretor::{Interpreter, Value};
    for body in ["|| item", "return || item;"] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Item {{}} struct Local {{}} impl Retire for Item {{fn drop(*self) {{}}}} impl Retire for Local {{fn drop(*self) {{}}}} typealias Callback = %{{item:Item}} fn()->Item; fn make()->Callback {{let local=Local {{}};let item=Item {{}};{body}}} fn entry() {{let callback=make();let result=callback();}}"
        ));
        let executable = source_executable(&package, "entry").unwrap();
        println!("NAMED_RETURN_DUMP={executable}\nNAMED_RETURN_END");
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Unit
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "make", "drop Local", "drop Item"]
        );
    }
}

#[test]
fn scheduled_named_return_preserves_callback_parameter_identity() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} typealias Callback = %{item:Item} fn()->Item; fn keep(callback:Callback)->Callback {callback} fn entry() {let item=Item {};let callback=keep(|| item);let result=callback();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "keep", "drop Item"]
    );
}

#[test]
fn scheduled_named_return_generic_factory_and_forwarding() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn make(value:T)->%{value:T} fn()->T where T {|| value} fn forward(value:Item)->%{value:Item} fn()->Item {make(value)} fn entry() {let callback=forward(Item {});let result=callback();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "forward", "make", "drop Item"]
    );
}

#[test]
fn scheduled_named_return_passes_factory_result_to_higher_order_function() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Callback=fn()->bool; fn selected()->bool {true} fn make()->Callback {selected} fn apply(callback:Callback)->bool {callback()} fn entry()->bool {apply(make())}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "apply", "selected"]
    );
}

#[test]
fn scheduled_named_return_postcondition_failure_cleans_undelivered_environment() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let mut package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} struct Caller {} struct Local {} impl Retire for Item {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} typealias Callback=%{item:Item} fn()->Item; fn make(flag:bool)->result:Callback where ensures true {let local=Local {};let item=Item {};|| item} fn entry(flag:bool) {let caller=Caller {};let callback=make(flag);let result=callback();}",
    );
    // Exercise a checked contract's rejection path after source admission.
    let make = package
        .functions
        .iter_mut()
        .find(|f| f.name == "make")
        .unwrap();
    make.postconditions[0].kind = nir::program::ExprKind::Local(make.parameters[0]);
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "make", "drop Local", "drop Item", "drop Caller"]
        );
    }
}

#[test]
fn scheduled_named_return_rejects_reusing_returned_affine_capture() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} typealias Callback=%{item:Item} fn()->Item; fn make()->Callback {let item=Item {};|| item} fn entry() {let callback=make();let first=callback();let second=callback();}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(
        error.contains("move") || error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn scheduled_named_return_keeps_same_typed_callback_inputs_separate() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Callback=fn()->bool; fn keep(callback:T)->T where T {callback} fn entry()->bool {let first=keep(|| false);let second=keep(|| true);let a=first();let b=second();not a and b}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "keep", "keep"]
    );
}

#[test]
fn scheduled_immediate_call_returned_closure_consumes_capture_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} typealias Callback=%{item:Item} fn()->Item; fn make()->Callback {let item=Item {};|| item} fn entry() {let result=make()();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("IMMEDIATE_CALL_DUMP={executable}\nIMMEDIATE_CALL_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "drop Item"]
    );
}

#[test]
fn scheduled_immediate_call_cleans_remaining_environment_after_call() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} struct After {} impl Retire for Item {fn drop(*self) {}} impl Retire for After {fn drop(*self) {}} typealias Callback=%{item:Item} fn()->bool; fn make()->Callback {let item=Item {flag:true};|| item.flag} fn entry()->bool {let result=make()();let after=After {};result}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "drop Item", "drop After"]
    );
}

#[test]
fn scheduled_immediate_call_literal_lambda_cleans_argument() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry() {(|item:Item| {})(Item {});}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item"]
    );
}

#[test]
fn scheduled_immediate_call_returned_named_function_retains_context() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Callback=fn()->bool; typealias Apply=fn(Callback)->bool; fn apply(callback:Callback)->bool {callback()} fn make()->Apply {apply} fn entry()->bool {make()(|| true)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "apply"]
    );
}

#[test]
fn scheduled_immediate_call_environment_outlives_outer_call_and_discarded_result() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Result {} struct Kept {flag:bool,} impl Retire for Result {fn drop(*self) {}} impl Retire for Kept {fn drop(*self) {}} typealias Callback=%{kept:Kept} fn()->Result; fn make()->Callback {let kept=Kept {flag:true};|| {let flag=kept.flag;Result {}}} fn consume(value:Result) {} fn entry() {consume(make()());make()();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "make",
            "consume",
            "drop Result",
            "drop Kept",
            "make",
            "drop Result",
            "drop Kept"
        ]
    );
}

#[test]
fn scheduled_immediate_call_argument_failure_cleans_prepared_values_then_environment() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Held {flag:bool,} struct Arg {} struct Caller {} impl Retire for Held {fn drop(*self) {}} impl Retire for Arg {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} typealias Callback=%{held:Held} fn(Arg,bool); fn make()->Callback {let held=Held {flag:true};|arg:Arg,flag:bool| {let flag=held.flag;}} fn fail()->bool {assumes false;true} fn entry() {let caller=Caller {};make()(Arg {},fail());}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "make",
            "fail",
            "drop Arg",
            "drop Held",
            "drop Caller"
        ]
    );
}

#[test]
fn scheduled_immediate_call_body_failure_cleans_only_remaining_captures() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Taken {} struct Held {flag:bool,} struct Local {} struct Caller {} impl Retire for Taken {fn drop(*self) {}} impl Retire for Held {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} typealias Callback=%{taken:Taken,held:Held} fn(); fn take(value:Taken) {} fn make(flag:bool)->Callback {let taken=Taken {};let held=Held {flag:flag};|| {take(taken);let local=Local {};let flag=held.flag;assumes flag;}} fn entry(flag:bool) {let caller=Caller {};make(flag)();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            [
                "entry",
                "make",
                "take",
                "drop Taken",
                "drop Local",
                "drop Held",
                "drop Caller"
            ]
        );
    }
}

#[test]
fn scheduled_immediate_call_proves_an_empty_pointer_footprint() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn entry() {let flag=true;(|pointer:*bool| {})(flag.ref);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut vm = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Unit);
}

#[test]
fn scheduled_immediate_call_argument_return_cleans_callee_without_invoking_it() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Held {flag:bool,} struct Caller {} impl Retire for Held {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} typealias Callback=%{held:Held} fn(bool)->bool; fn make()->Callback {let held=Held {flag:true};|flag:bool| held.flag} fn entry()->bool {let caller=Caller {};make()(do {return true;});false}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "drop Held", "drop Caller"]
    );
}

#[test]
fn scheduled_immediate_call_failed_environment_drop_stops_caller_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Held {flag:bool,} struct Caller {} impl Retire for Held {fn drop(*self) {assumes false;}} impl Retire for Caller {fn drop(*self) {}} typealias Callback=%{held:Held} fn()->bool; fn make()->Callback {let held=Held {flag:true};|| held.flag} fn consume(flag:bool) {} fn entry() {let caller=Caller {};consume(make()());}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "consume", "drop Held"]
    );
}

#[test]
fn scheduled_immediate_call_loop_reuses_storage_without_retaining_old_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Held {flag:bool,} impl Retire for Held {fn drop(*self) {}} typealias Callback=%{held:Held} fn()->bool; fn make()->Callback {let held=Held {flag:true};|| held.flag} fn entry() {var again=true;var last=false;while again {make()();again=not last;last=true;}}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter.invoke_instance(&executable.entry, vec![]).unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "drop Held", "make", "drop Held"]
    );
}

#[test]
fn scheduled_immediate_call_chain_transfers_environment_to_next_callee() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} typealias Inner=%{item:Item} fn()->Item; typealias Outer=%{item:Item} fn()->Inner; fn make()->Outer {let item=Item {};|| {|| item}} fn entry() {let result=make()()();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter.invoke_instance(&executable.entry, vec![]).unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "drop Item"]
    );
}

#[test]
fn scheduled_owned_default_result_transfers_to_caller_without_early_cleanup() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} struct Later {} impl Retire for Item {fn drop(*self) {}} impl Retire for Later {fn drop(*self) {}} fn obtain(.item:Item=do {Item {}})->Item {item} fn entry() {let item=obtain();let later=Later {};}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    println!("OWNED_DEFAULT_DUMP={executable}\nOWNED_DEFAULT_END");
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "obtain", "drop Later", "drop Item"]
    );
}

#[test]
fn scheduled_owned_defaults_are_fresh_per_call_and_keep_nested_cleanup_order() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Left {} struct Right {} struct Pair {left:Left,right:Right,} impl Retire for Left {fn drop(*self) {}} impl Retire for Right {fn drop(*self) {}} impl Retire for Pair {fn drop(*self) {}} fn consume(.pair:Pair=do {Pair {left:Left {},right:Right {}}}) {} fn entry() {var again=true;var last=false;while again {consume();again=not last;last=true;}}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        [
            "entry",
            "consume",
            "drop Pair",
            "drop Right",
            "drop Left",
            "consume",
            "drop Pair",
            "drop Right",
            "drop Left"
        ]
    );
}

#[test]
fn scheduled_indirect_pointer_result_retains_origins_and_writes() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn alias(value:*bool)->*bool {value} fn entry()->bool {let value=false;let helper=alias;let pointer=helper(value.ref);pointer.* = true;value}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "alias"]
    );
}

#[test]
fn scheduled_indirect_pointer_result_snapshots_callee_before_arguments() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first(a:*bool,b:*bool)->*bool {a} fn second(a:*bool,b:*bool)->*bool {b} fn entry()->bool {let a=true;let b=false;var helper=first;let pointer=helper(a.ref,do {helper=second;b.ref});pointer.*}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "first"]
    );
}

#[test]
fn scheduled_indirect_pointer_result_cannot_outlive_moved_owner() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} fn alias(value:*Item)->*Item {value} fn entry()->bool {let item=Item {flag:true};let helper=alias;let pointer=helper(item.ref);let moved=item;pointer.flag.*}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn scheduled_indirect_pointer_result_keeps_generic_and_factory_targets() {
    use comptime_interpretor::{Interpreter, Value};
    for source in [
        "fn alias(value:*T)->*T where T {value} fn entry()->bool {let value=true;let helper:fn(*bool)->*bool=alias;let pointer=helper(value.ref);pointer.*}",
        "typealias Alias=fn(*bool)->*bool; fn alias(value:*bool)->*bool {value} fn make()->Alias {alias} fn entry()->bool {let value=true;let helper=make();let pointer=helper(value.ref);pointer.*}",
    ] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} {source}"
        ));
        let executable = source_executable(&package, "entry").unwrap();
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Bool(true)
        );
        let events = executed_names(&package, &executable, interpreter.events());
        assert_eq!(
            events
                .iter()
                .filter(|name| name.as_str() == "alias")
                .count(),
            1,
            "{events:?}"
        );
    }
}

#[test]
fn scheduled_temporary_pointer_call_retains_targets_and_evaluates_factory_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Alias=fn(*bool)->*bool; fn alias(value:*bool)->*bool {value} fn make()->Alias {alias} fn entry()->bool {let value=false;let pointer=make()(value.ref);pointer.* = true;value}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "alias"]
    );
}

#[test]
fn scheduled_temporary_pointer_call_cannot_read_moved_pointee() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} typealias Alias=fn(*Item)->*Item; fn alias(value:*Item)->*Item {value} fn make()->Alias {alias} fn entry()->bool {let item=Item {flag:true};let pointer=make()(item.ref);let moved=item;pointer.flag.*}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn scheduled_temporary_pointer_call_keeps_callable_result_with_existing_borrows() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Alias=fn(*bool)->*bool; fn alias(value:*bool)->*bool {value} fn make()->Alias {alias} fn entry()->bool {let value=true;let original=value.ref;let pointer=make()(original);pointer.*}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "alias"]
    );
}

#[test]
fn scheduled_temporary_pointer_call_argument_return_cleans_caller_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} impl Retire for Caller {fn drop(*self) {}} typealias Alias=fn(*bool)->*bool; fn alias(value:*bool)->*bool {value} fn make()->Alias {alias} fn entry()->bool {let caller=Caller {};let pointer=make()(do {return true;});false}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "drop Caller"]
    );
}

#[test]
fn scheduled_temporary_pointer_call_preserves_captured_pointer_origins() {
    use comptime_interpretor::{Interpreter,Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn entry()->bool {let value=true;let pointer=value.ref;let returned=(|| pointer)();returned.*}",
    );
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    let escaping=compile("trait Retire {fn drop(*self);} trait Duplicate {} fn entry()->*bool {let value=true;let pointer=value.ref;(|| pointer)()}");
    let error=source_executable(&escaping,"entry").unwrap_err();
    assert!(error.contains("escaping provenance"),"{error}");
}

#[test]
fn scheduled_memory_wrapper_calls_local_named_alias_and_preserves_writes() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn set(value:*bool)->*bool {value.* = true;value} fn wrapper(value:*bool)->*bool {let helper=set;helper(value)} fn entry()->bool {let value=false;let pointer=wrapper(value.ref);pointer.*}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "wrapper", "set"]
    );
}

#[test]
fn scheduled_memory_callable_expression_calls_checked_target() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn set(value:*bool)->*bool {value.* = true;value} fn wrapper(value:*bool,flag:bool)->*bool {(if flag {set} else {set})(value)} fn entry()->bool {let value=false;let pointer=wrapper(value.ref,true);pointer.*}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "wrapper", "set"]
    );
}

#[test]
fn scheduled_memory_callable_expression_preserves_callee_effects_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn alias(value:*bool)->*bool {value} fn wrapper(value:*bool)->*bool {(do {value.* = not value.*;alias})(value)} fn entry()->bool {let value=false;let pointer=wrapper(value.ref);pointer.*}",
    );
    let wrapper = package.functions.iter().find(|f| f.name == "wrapper").unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package, wrapper.declaration, &[], &mut 100000,
    ).unwrap();
    assert!(summary.accesses.iter().any(|access| access.write && access.target.parameter == 0), "{summary}");
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(interpreter.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "wrapper", "alias"]);
}

#[test]
fn scheduled_memory_factory_wrapper_keeps_indirect_writes_and_result_origins() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Setter=fn(*bool)->*bool; fn set(value:*bool)->*bool {value.* = true;value} fn make()->Setter {set} fn wrapper(value:*bool)->*bool {make()(value)} fn entry()->bool {let value=false;let pointer=wrapper(value.ref);pointer.*}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "wrapper", "make", "set"]
    );
}

#[test]
fn scheduled_named_candidates_select_one_body_and_keep_all_pointer_dependencies() {
    use comptime_interpretor::{Interpreter, Value};
    for body in [
        "let helper=if flag {first} else {second};let pointer=helper(a.ref,b.ref);pointer.*",
        "let helper=flag match {true=>first,false=>second,};let pointer=helper(a.ref,b.ref);pointer.*",
        "let pointer=(if flag {first} else {second})(a.ref,b.ref);pointer.*",
    ] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn first(a:*bool,b:*bool)->*bool {{a}} fn second(a:*bool,b:*bool)->*bool {{b}} fn entry(flag:bool)->bool {{let a=true;let b=false;{body}}}"
        ));
        let executable = source_executable(&package, "entry").unwrap();
        for flag in [true, false] {
            let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
                .unwrap()
                .with_cleanup_schedule(&executable.schedule)
                .unwrap();
            assert_eq!(
                interpreter
                    .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                    .unwrap(),
                Value::Bool(flag)
            );
            assert_eq!(
                executed_names(&package, &executable, interpreter.events()),
                ["entry", if flag { "first" } else { "second" }]
            );
        }
    }
}

#[test]
fn scheduled_named_candidates_check_every_possible_returned_owner() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} fn first(a:*Item,b:*Item)->*Item {a} fn second(a:*Item,b:*Item)->*Item {b} fn entry(flag:bool)->bool {let a=Item {flag:true};let b=Item {flag:false};let helper=if flag {first} else {second};let pointer=helper(a.ref,b.ref);let moved=a;pointer.flag.*}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn scheduled_named_candidates_argument_evaluation_happens_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first(value:bool)->bool {value} fn second(value:bool)->bool {not value} fn entry(flag:bool)->bool {var value=false;let helper=if flag {first} else {second};helper(do {value=not value;value})}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(flag)
        );
    }
}

#[test]
fn scheduled_named_candidates_rejection_cleans_caller_and_stops_selected_body() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} impl Retire for Caller {fn drop(*self) {}} fn fail() {assumes false;} fn pass() {} fn entry(flag:bool) {let caller=Caller {};let helper=if flag {fail} else {pass};helper();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        } else {
            assert_eq!(result.unwrap(), Value::Unit);
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", if flag { "fail" } else { "pass" }, "drop Caller"]
        );
    }
}

#[test]
fn scheduled_named_candidates_loop_converges_without_losing_reassignment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first()->bool {false} fn second()->bool {true} fn entry(flag:bool)->bool {var helper=first;var again=flag;while again {helper=second;again=false;}helper()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(flag)
        );
    }
}

#[test]
fn scheduled_named_candidates_factory_returns_the_complete_set() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Callback=fn()->bool; fn first()->bool {true} fn second()->bool {false} fn make(flag:bool)->Callback {if flag {first} else {second}} fn entry(flag:bool)->bool {make(flag)()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(flag)
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "make", if flag { "first" } else { "second" }]
        );
    }
}

#[test]
fn scheduled_named_candidates_chained_factory_calls_keep_returned_targets() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Callback=fn()->bool; fn first()->bool {true} fn second()->bool {false} fn make_first()->Callback {first} fn make_second()->Callback {second} fn entry(flag:bool)->bool {let factory=if flag {make_first} else {make_second};factory()()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(flag)
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            [
                "entry",
                if flag { "make_first" } else { "make_second" },
                if flag { "first" } else { "second" }
            ]
        );
    }
}

#[test]
fn scheduled_named_candidates_higher_order_call_forward_and_return() {
    use comptime_interpretor::{Interpreter, Value};
    for body in [
        "apply(helper)",
        "forward(helper)",
        "returned(helper)()",
        "apply(returned(helper))",
        "apply(if flag {first} else {second})",
    ] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} typealias Callback=fn()->bool; fn first()->bool {{true}} fn second()->bool {{false}} fn apply(callback:Callback)->bool {{callback()}} fn forward(callback:Callback)->bool {{apply(callback)}} fn returned(callback:Callback)->Callback {{callback}} fn entry(flag:bool)->bool {{let helper=if flag {{first}} else {{second}};{body}}}",
        ));
        let executable = source_executable(&package, "entry").unwrap_or_else(|e| panic!("{body}: {e}"));
        for flag in [true, false] {
            let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]).unwrap(), Value::Bool(flag), "{body}");
            let names = executed_names(&package, &executable, interpreter.events());
            assert_eq!(names.iter().filter(|name| **name == "first" || **name == "second").count(), 1, "{names:?}");
            assert!(names.contains(&if flag {"first"} else {"second"}.to_owned()), "{names:?}");
        }
    }
}

#[test]
fn scheduled_named_candidates_keep_instantiated_generic_targets() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first(a:*T,b:*T)->*T where T {a} fn second(a:*T,b:*T)->*T where T {b} fn entry(flag:bool)->bool {let a=true;let b=false;let helper:fn(*bool,*bool)->*bool=if flag {first} else {second};let pointer=helper(a.ref,b.ref);pointer.*}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]).unwrap(), Value::Bool(flag));
    }
}

#[test]
fn scheduled_named_candidates_higher_order_failure_cleans_each_frame_once() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Frame {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Frame {fn drop(*self) {}} fn fail() {assumes false;} fn pass() {} fn apply(callback:fn()) {let frame=Frame {};callback();} fn entry(flag:bool) {let caller=Caller {};let callback=if flag {fail} else {pass};apply(callback);}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        } else {
            assert_eq!(result.unwrap(), Value::Unit);
        }
        assert_eq!(executed_names(&package, &executable, interpreter.events()),
            ["entry", "apply", if flag {"fail"} else {"pass"}, "drop Frame", "drop Caller"]);
    }
}

#[test]
fn scheduled_named_candidates_higher_order_preparation_runs_once() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first(value:bool)->bool {value} fn second(value:bool)->bool {not value} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} fn entry(flag:bool)->bool {var value=false;apply(do {value=not value;if flag {first} else {second}},value)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]).unwrap(), Value::Bool(flag));
        assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "apply", if flag {"first"} else {"second"}]);
    }
}

#[test]
fn scheduled_named_candidates_higher_order_loop_publishes_converged_context() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first()->bool {false} fn second()->bool {true} fn apply(callback:fn()->bool)->bool {callback()} fn entry()->bool {var helper=first;var done=false;var result=false;while not done {result=apply(helper);helper=second;if result {done=true;}}result}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(interpreter.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "apply", "first", "apply", "second"]);
}

#[test]
fn scheduled_named_candidates_higher_order_default_keeps_all_targets() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Callback=fn()->bool; fn first()->bool {true} fn second()->bool {false} fn apply(flag:bool,.callback:Callback=do {if flag {first} else {second}})->bool {callback()} fn entry(flag:bool)->bool {apply(flag)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]).unwrap(), Value::Bool(flag));
        assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "apply", if flag {"first"} else {"second"}]);
    }
}

#[test]
fn scheduled_named_candidates_higher_order_generic_instances_remain_concrete() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first(value:T)->bool where T {true} fn second(value:T)->bool where T {false} fn apply(callback:fn(bool)->bool)->bool {callback(false)} fn entry(flag:bool)->bool {let helper:fn(bool)->bool=if flag {first} else {second};apply(helper)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]).unwrap(), Value::Bool(flag));
        assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "apply", if flag {"first"} else {"second"}]);
    }
}

#[test]
fn scheduled_captured_candidates_survive_parameters_forwarding_and_factory_returns() {
    use comptime_interpretor::{Interpreter, Value};
    for body in [
        "let helper=if flag {first} else {second};let wrapper=|| helper();apply(wrapper)",
        "let helper=if flag {first} else {second};let wrapper=|| helper();forward(wrapper)",
        "make(flag)()",
        "apply(make(flag))",
        "let helper=if flag {first} else {second};let wrapper=|| helper();returned(wrapper)()",
    ] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} typealias Code=fn()->bool; typealias Wrapper=%{{helper:Code}} fn()->bool; fn first()->bool {{true}} fn second()->bool {{false}} fn apply(wrapper:Wrapper)->bool {{wrapper()}} fn forward(wrapper:Wrapper)->bool {{apply(wrapper)}} fn returned(wrapper:Wrapper)->Wrapper {{wrapper}} fn make(flag:bool)->Wrapper {{let helper=if flag {{first}} else {{second}};|| helper()}} fn entry(flag:bool)->bool {{{body}}}"
        ));
        let executable =
            source_executable(&package, "entry").unwrap_or_else(|e| panic!("{body}: {e}"));
        for flag in [true, false] {
            let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
                .unwrap()
                .with_cleanup_schedule(&executable.schedule)
                .unwrap();
            assert_eq!(
                interpreter
                    .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                    .unwrap(),
                Value::Bool(flag),
                "{body}"
            );
            let names = executed_names(&package, &executable, interpreter.events());
            assert_eq!(
                names
                    .iter()
                    .filter(|name| **name == "first" || **name == "second")
                    .count(),
                1,
                "{names:?}"
            );
            assert!(
                names.contains(&if flag { "first" } else { "second" }.to_owned()),
                "{names:?}"
            );
        }
    }
}

#[test]
fn scheduled_captured_candidates_recursive_environments_keep_all_targets() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Code=fn()->bool; typealias Inner=%{helper:Code} fn()->bool; typealias Outer=%{inner:Inner} fn()->bool; fn first()->bool {true} fn second()->bool {false} fn make(flag:bool)->Outer {let helper=if flag {first} else {second};let inner=|| helper();|| inner()} fn apply(outer:Outer)->bool {outer()} fn entry(flag:bool)->bool {apply(make(flag))}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(flag)
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            [
                "entry",
                "make",
                "apply",
                if flag { "first" } else { "second" }
            ]
        );
    }
}

#[test]
fn scheduled_captured_candidates_rejection_cleans_environment_and_caller_once() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Held {flag:bool,} struct Caller {} impl Retire for Held {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} typealias Code=fn()->bool; typealias Wrapper=%{held:Held,helper:Code} fn()->bool; fn first()->bool {assumes false;true} fn second()->bool {false} fn make(flag:bool)->Wrapper {let held=Held {flag:true};let helper=if flag {first} else {second};|| {let flag=held.flag;helper()}} fn apply(wrapper:Wrapper)->bool {wrapper()} fn entry(flag:bool)->bool {let caller=Caller {};apply(make(flag))}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        let result = interpreter.invoke_instance(&executable.entry, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        } else {
            assert_eq!(result.unwrap(), Value::Bool(false));
        }
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            [
                "entry",
                "make",
                "apply",
                if flag { "first" } else { "second" },
                "drop Held",
                "drop Caller"
            ]
        );
    }
}

#[test]
fn scheduled_captured_candidates_do_not_restore_consumed_capture_state() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {} impl Retire for Item {fn drop(*self) {}} typealias Code=fn()->bool; typealias Wrapper=%{item:Item,helper:Code} fn()->bool; fn first()->bool {true} fn second()->bool {false} fn make(flag:bool)->Wrapper {let item=Item {};let helper=if flag {first} else {second};|| {let consumed=item;helper()}} fn apply(wrapper:Wrapper)->bool {wrapper();wrapper()} fn entry(flag:bool)->bool {apply(make(flag))}",
    );
    let error = source_executable(&package, "entry").unwrap_err();
    assert!(
        error.contains("unavailable") || error.contains("move"),
        "{error}"
    );
}

#[test]
fn scheduled_captured_callable_replacement_joins_explicit_return_paths() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first()->bool {true} fn second()->bool {false} fn third()->bool {true} fn entry(flag:bool)->bool {var inner=first;let outer=|flag:bool| {let previous=inner();if flag {inner=second;return previous;}inner=third;previous};outer(flag);outer(false)}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    for flag in [true, false] {
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(!flag)
        );
        assert_eq!(
            executed_names(&package, &executable, interpreter.events()),
            ["entry", "first", if flag { "second" } else { "third" }]
        );
    }
}

#[test]
fn scheduled_captured_callable_replacement_retires_old_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} fn entry()->bool {let item=Item {flag:true};var inner=|| item.flag;let outer=|| {inner=do {let item=Item {flag:false};|| item.flag};inner()};outer()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item", "drop Item"]
    );
}

#[test]
fn scheduled_captured_callable_replacement_survives_returned_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} typealias Code=fn()->bool; typealias Wrapper=%{inner:Code} fn()->bool; fn first()->bool {true} fn second()->bool {false} fn make()->Wrapper {var inner=first;|| {let previous=inner();inner=second;previous}} fn update(wrapper:Wrapper)->Wrapper {wrapper();wrapper} fn entry()->bool {let wrapper=update(make());wrapper()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "make", "update", "first", "second"]
    );
}

#[test]
fn scheduled_captured_callable_replacement_failure_cleans_new_environment() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} struct Caller {} impl Retire for Item {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}} fn entry() {let caller=Caller {};let item=Item {flag:true};var inner=|| item.flag;let outer=|| {inner=do {let item=Item {flag:false};|| item.flag};assumes false;inner()};outer();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item", "drop Item", "drop Caller"]
    );
}

#[test]
fn scheduled_captured_callable_replacement_assignment_alone_acquires_capture() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} fn entry() {let item=Item {flag:true};var inner=|| item.flag;let outer=|| {inner=do {let item=Item {flag:false};|| item.flag};};outer();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item", "drop Item"]
    );
}

#[test]
fn scheduled_captured_callable_replacement_loop_converges() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} fn first()->bool {false} fn second()->bool {true} fn entry()->bool {var inner=first;let outer=|| {let previous=inner();inner=second;previous};var done=false;while not done {done=outer();}done}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "first", "second"]
    );
}

#[test]
fn scheduled_captured_callable_replacement_self_move_preserves_environment() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} fn entry()->bool {let item=Item {flag:true};var inner=|| item.flag;let outer=|| {inner=inner;inner()};outer()}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(interpreter.invoke_instance(&executable.entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(executed_names(&package, &executable, interpreter.events()), ["entry", "drop Item"]);
}

#[test]
fn scheduled_mutable_captures_keep_values_fields_bits_and_factory_state() {
    use comptime_interpretor::{Interpreter, Value};
    for source in [
        "fn entry()->bool {var flag=false;let toggle=|| {flag=not flag;flag};let first=toggle();let second=toggle();first and not second and not flag}",
        "fn entry()->bool {var flag=false;let left=|| {flag=not flag;flag};let right=|| {flag=not flag;flag};let first=left();let other=right();let second=left();first and other and not second and not flag}",
        "typealias Counter=%{flag:bool} fn()->bool; fn make()->Counter {var flag=false;|| {flag=not flag;flag}} fn apply(counter:Counter)->bool {let first=counter();let second=counter();first and not second} fn entry()->bool {apply(make())}",
        "struct State {flag:bool,} impl Retire for State {fn drop(*self) {let flag=self.flag.*;assumes not flag;}} fn entry()->bool {let state=State {flag:false};let toggle=|| {state.flag=not state.flag;state.flag};let first=toggle();let second=toggle();first and not second}",
        "fn entry()->bool {var bits:.u8'builtin=0;let write=|| {bits'bitvec[0..4]=3;bits'bitvec[4..8]=10;bits==163};write() and bits==0}",
        "fn entry()->bool {var flag=false;let outer=|| {flag=not flag;|| flag};let first=outer();let second=outer();first() and not second()}",
        "fn entry()->bool {var flag=false;let write=|| {flag=true;};write();not flag}",
    ] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} {source}"
        ));
        let executable =
            source_executable(&package, "entry").unwrap_or_else(|e| panic!("{source}: {e}"));
        let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(&executable.schedule)
            .unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&executable.entry, vec![])
                .unwrap(),
            Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn scheduled_mutable_capture_replacement_and_rejection_observe_updated_resources() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {ready:bool,} struct Caller {} impl Retire for Item {fn drop(*self) {let ready=self.ready.*;assumes ready;}} impl Retire for Caller {fn drop(*self) {}} struct Pair {item:Item,} fn entry() {let caller=Caller {};let pair=Pair {item:Item {ready:true}};let update=|| {pair.item=Item {ready:false};pair.item.ready=true;assumes false;};update();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap_err()
            .kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item", "drop Item", "drop Caller"]
    );
}

#[test]
fn scheduled_mutable_capture_pointer_replacement_requires_memory_evidence() {
    for source in [
        "fn entry() {let source=true;var pointer=source.ref;let replace=|| {let local=false;pointer=local.ref;};replace();}",
        "struct Holder {pointer:*bool,} fn entry() {let source=true;var holder=Holder {pointer:source.ref};let replace=|| {let local=false;holder=Holder {pointer:local.ref};};replace();}",
    ] {
        let package = compile(&format!(
            "trait Retire {{fn drop(*self);}} trait Duplicate {{}} {source}"
        ));
        let error = source_executable(&package, "entry").unwrap_err();
        assert!(
            error.contains("captured write requires an established memory contract"),
            "{error}"
        );
    }
}

#[test]
fn scheduled_mutable_capture_reinitializes_moved_field_before_next_call() {
    use comptime_interpretor::{Interpreter, Value};
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Item {ready:bool,} impl Retire for Item {fn drop(*self) {let ready=self.ready.*;assumes ready;}} struct Pair {item:Item,} fn entry() {let pair=Pair {item:Item {ready:true}};let renew=|| {let old=pair.item;pair.item=Item {ready:true};};renew();renew();}",
    );
    let executable = source_executable(&package, "entry").unwrap();
    let mut interpreter = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        interpreter
            .invoke_instance(&executable.entry, vec![])
            .unwrap(),
        Value::Unit
    );
    assert_eq!(
        executed_names(&package, &executable, interpreter.events()),
        ["entry", "drop Item", "drop Item", "drop Item"]
    );
}

#[test]
fn variadic_cleanup_transfers_packages_and_unwinds_partial_collection_once() {
    use comptime_interpretor::{Interpreter,Value};
    for (body,expected) in [
        ("take(Left {},Right {});",vec!["entry","take","drop Right","drop Left"]),
        ("take(.items=(Left {},Right {}));",vec!["entry","take","drop Right","drop Left"]),
        ("take(Left {},.flag=do {return;false},Right {});",vec!["entry","drop Left"]),
        ("take(Left {},do {return;Right {}});",vec!["entry","drop Left"]),
        ("let held=keep(Left {},Right {});",vec!["entry","keep","drop Right","drop Left"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn take(...items:(Left,Right),.flag:bool=false) {{}} fn keep(...items:T)->T where T {{items}} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn variadic_collection_preserves_temporary_owner_cleanup_boundaries() {
    use comptime_interpretor::{Interpreter,Value};
    let mut traces=Vec::new();
    for input in ["take(make().first,Right {})", "take(.items=(make().first,Right {}))"] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} struct Extra {{}} struct Pair {{first:Left,extra:Extra}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} impl Retire for Extra {{fn drop(*self) {{}}}} fn make()->Pair {{Pair {{first:Left {{}},extra:Extra {{}}}}}} fn take(...items:(Left,Right)) {{}} fn entry() {{{input};}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        traces.push(executed_names(&package,&executable,vm.events()));
    }
    assert_eq!(traces[0],traces[1],"collection must not shorten a projected temporary owner's lifetime");
}

#[test]
fn variadic_partial_projection_cleanup_matches_direct_package_on_exit_and_rejection() {
    use comptime_interpretor::Interpreter;
    for rejected in [false,true] {
        let mut traces=Vec::new();
        for collected in [false,true] {
            let tail=if rejected {"Right {}"} else {"do {return;Right {}}"};
            let input=if collected {format!("take(make().first,{tail})")} else {format!("take(.items=(make().first,{tail}))")};
            let predicate=if rejected {"assumes flag;"} else {""};
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} struct Extra {{}} struct Pair {{first:Left,extra:Extra}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} impl Retire for Extra {{fn drop(*self) {{}}}} fn make()->Pair {{Pair {{first:Left {{}},extra:Extra {{}}}}}} fn take(...items:(Left,Right),.flag:bool=false) {{{predicate}}} fn entry() {{{input};}}"));
            let executable=source_executable(&package,"entry").unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).is_err(),rejected);
            traces.push(executed_names(&package,&executable,vm.events()));
        }
        assert_eq!(traces[0],traces[1],"rejected={rejected}");
        assert_eq!(traces[0].iter().filter(|name|name.as_str()=="drop Left").count(),1);
        assert_eq!(traces[0].iter().filter(|name|name.as_str()=="drop Extra").count(),1);
        assert_eq!(traces[0].iter().filter(|name|name.as_str()=="drop Right").count(),usize::from(rejected));
    }
}

#[test]
fn tuple_spreads_transfer_affine_packets_and_clean_partial_preparation_once() {
    use comptime_interpretor::{Interpreter,Value};
    for (input,expected) in [
        ("take(...packet())",vec!["entry","packet","take","drop Right","drop Left"]),
        ("take(...packet(),.flag=do {return;false})",vec!["entry","packet","drop Right","drop Left"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn packet()->(Left,Right) {{(Left {{}},Right {{}})}} fn take(...items:(Left,Right),.flag:bool=false) {{}} fn entry() {{{input};}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        assert!(result.is_ok(),"{input}: {result:?}, events={:?}, executable={executable}",vm.events());
        assert_eq!(result.unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{input}");
    }
}

#[test]
fn tuple_spreads_contextual_receiver_packets_preserve_early_exit_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    for (body,expected) in [
        ("make().take(...do {(7,true)});",vec!["entry","make","take","drop Holder"]),
        ("make().take(...do {return;(7,true)});",vec!["entry","make","drop Holder"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Holder {{}} impl Retire for Holder {{fn drop(*self) {{}}}} impl Holder {{fn take(self,number:.u8'builtin,flag:bool)->bool {{flag}}}} fn make()->Holder {{Holder {{}}}} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn tuple_spreads_multiple_contextual_packets_clean_only_initialized_inputs() {
    use comptime_interpretor::{Interpreter, Value};
    for (second, expected) in [
        ("do {(9,Right {})}", vec!["entry","take","drop Right","drop Left"]),
        ("do {return;(9,Right {})}", vec!["entry","drop Left"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn take(a:.u8'builtin,left:Left,b:.u16'builtin,right:Right) {{}} fn entry() {{take(...do {{(7,Left {{}})}},...{second});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{second}");
    }
}

#[test]
fn constructor_spreads_transfer_fields_and_clean_partial_payloads_once() {
    use comptime_interpretor::{Interpreter, Value};
    for (tail,expected) in [
        ("do {(9,Right {})}",vec!["entry","drop Right","drop Left"]),
        ("do {return;(9,Right {})}",vec!["entry","drop Left"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} enum Choice {{pair(.u8'builtin,Left,.u16'builtin,Right)}} fn entry() {{let value=Choice.pair(...do {{(7,Left {{}})}},...{tail});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{tail}");
    }
}

#[test]
fn constructor_spreads_preserve_projected_owner_lifetimes_and_failure_cleanup() {
    use comptime_interpretor::{ErrorKind, Interpreter};
    for rejected in [false,true] {
        let mut traces=Vec::new();
        for spread in [false,true] {
            let tail=if rejected {"do {assumes false;Right {}}"} else {"Right {}"};
            let input=if spread {format!("Choice.pair(...do {{(make().first,)}},...do {{({tail},)}})")} else {format!("Choice.pair(make().first,{tail})")};
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} struct Extra {{}} struct Pair {{first:Left,extra:Extra}} enum Choice {{pair(Left,Right)}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} impl Retire for Extra {{fn drop(*self) {{}}}} fn make()->Pair {{Pair {{first:Left {{}},extra:Extra {{}}}}}} fn entry() {{let result={input};}}"));
            let executable=source_executable(&package,"entry").unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            let result=vm.invoke_instance(&executable.entry,vec![]);
            if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert!(result.is_ok(),"{result:?}");}
            traces.push(executed_names(&package,&executable,vm.events()));
        }
        // A source do-block has its own temporary boundary, so compare actual
        // retirement counts rather than equating it to a direct projection.
        for trace in traces {
            assert_eq!(trace.iter().filter(|name|name.as_str()=="drop Left").count(),1,"{trace:?}");
            assert_eq!(trace.iter().filter(|name|name.as_str()=="drop Extra").count(),1,"{trace:?}");
            assert_eq!(trace.iter().filter(|name|name.as_str()=="drop Right").count(),usize::from(!rejected),"{trace:?}");
        }
    }
}

#[test]
fn extended_calls_preserve_affine_delivery_and_partial_input_cleanup() {
    use comptime_interpretor::{Interpreter, Value};
    for (body, expected) in [
        ("take {Left {},Right {}};",vec!["entry","take","drop Right","drop Left"]),
        ("take {Left {},do {return;Right {}}};",vec!["entry","drop Left"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn take(...items:(Left,Right)) {{}} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn record_cleanup_plans_nested_fields_and_skips_transferred_responsibilities() {
    use elaboration::verification::{drop::{plan_cleanup,CleanupStep},ownership::{State,Shape,Place},PlaceId};
    let package=compile("trait Retire {fn drop(*self);} struct Left {} struct Right {} impl Retire for Left {fn drop(*self) {}} impl Retire for Right {fn drop(*self) {}}");
    world(&package,|world,items| {
        let ty=nir::Term::Record(vec![("left".into(),nominal(&package,"Left",vec![])),("nested".into(),nir::Term::Record(vec![("right".into(),nominal(&package,"Right",vec![]))]))]);
        let acquired=luna::protocols::acquire_cleanup(&package,world,items,&ty,Budget::default(),&mut 10000).unwrap();
        let root=PlaceId {owner:1,local:1};let receiver=Place {root,fields:vec![]};
        let mut state=State::default();
        state.declare(root,&Shape::Fields(vec![Shape::Fields(vec![]),Shape::Fields(vec![Shape::Fields(vec![])])]),true,&mut 1000).unwrap();
        for expected in [vec![vec![1,0],vec![0]],vec![vec![0]]] {
            let before=state.clone();
            let plan=plan_cleanup(&package,world,&acquired,&state,&receiver,&mut 10000).unwrap();
            let paths:Vec<_>=plan.steps.iter().filter_map(|s|if let CleanupStep::Invoke {access,..}=s {Some(access.receiver.fields.clone())}else{None}).collect();
            assert_eq!(paths,expected,"{plan}");
            assert_eq!(state,before,"planning must not retire resources before execution");
            if expected.len()==2 {state.take(&Place {root,fields:vec![1,0]}).unwrap();}
        }
    });
}

#[test]
fn record_ir_executes_acquired_cleanup_for_complete_and_partial_construction() {
    use comptime_interpretor::{Interpreter,Value};
    use nir::program::ExprKind as K;
    for (tail,expected) in [
        ("Right {}",vec!["entry","drop Right","drop Left"]),
        ("do {return;Right {}}",vec!["entry","drop Left"]),
    ] {
        let mut package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} struct Bundle {{first:Left,second:Right}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn entry() {{let value=Bundle {{first:Left {{}},second:{tail}}};}}"));
        let record=nir::Term::Record(vec![("first".into(),nominal(&package,"Left",vec![])),("second".into(),nominal(&package,"Right",vec![]))]);
        let entry=package.functions.iter_mut().find(|f|f.name=="entry").unwrap();
        let K::Block {statements,..}=&mut entry.body.kind else {panic!("block")};
        let K::Let {value,..}=&mut statements[0].kind else {panic!("binding")};
        let K::Record {fields,..}=std::mem::replace(&mut value.kind,K::Unit) else {panic!("construction")};
        value.kind=K::AnonymousRecord {fields};
        if value.ty!=nir::Term::Primitive(nir::Primitive::Never) {value.ty=record;}
        // This is an IR fixture until source attribute collection is connected.
        nir::phases::validate(&package).unwrap();
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{tail}");
    }
}

#[test]
fn property_collection_delivers_owned_fields_and_cleans_partial_preparation() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (body,expected,rejected) in [
        ("let value=keep {first:Left {},second:Right {}};",vec!["entry","keep","drop Right","drop Left"],false),
        ("let value=keep {first:Left {},second:Right {}};let first=value.first;",vec!["entry","keep","drop Left","drop Right"],false),
        ("let first=keep {first:Left {},second:Right {}}.first;",vec!["entry","keep","drop Right","drop Left"],false),
        ("let value=keep {first:Left {},second:do {return;Right {}}};",vec!["entry","drop Left"],false),
        ("let value=keep {first:Left {},second:do {assumes false;Right {}}};",vec!["entry","drop Left"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn keep(...properties:T)->T where T {{properties}} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn method_properties_cleanup_receiver_and_partial_fields() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (second,expected,rejected) in [
        ("Right {}",vec!["entry","make","take","drop Right","drop Left","drop Holder"],false),
        ("do {return;Right {}}",vec!["entry","make","drop Left","drop Holder"],false),
        ("do {assumes false;Right {}}",vec!["entry","make","drop Left","drop Holder"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Holder {{}} struct Left {{}} struct Right {{}} impl Retire for Holder {{fn drop(*self) {{}}}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{first:Left {{}},second:Right {{}}}}; typealias Options=SAMPLE'type; impl Holder {{fn take(self,...properties:Options) {{}}}} fn make()->Holder {{Holder {{}}}} fn entry() {{make().take {{first:Left {{}},second:{second}}};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{second}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{second}");
    }
}

#[test]
fn anonymous_record_replacement_retires_only_completed_destination() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (replacement,expected,rejected) in [
        ("Left {}",vec!["entry","keep","drop Left","drop Right","drop Left"],false),
        ("do {value.second=Right {};Left {}}",vec!["entry","keep","drop Right","drop Left","drop Right","drop Left"],false),
        ("do {assumes false;Left {}}",vec!["entry","keep","drop Right","drop Left"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Left {{}} struct Right {{}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn keep(...properties:T)->T where T {{properties}} fn entry() {{let value=keep {{first:Left {{}},second:Right {{}}}};value.first={replacement};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{replacement}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{replacement}");
    }
}

#[test]
fn mixed_property_preparation_cleans_acquired_inputs_in_reverse_order() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (inputs,expected,rejected) in [
        ("head(),first:left(),tail(),second:right()",vec!["entry","head","left","tail","right","take","drop Tail","drop Right","drop Left","drop Head"],false),
        ("head(),first:left(),tail(),second:do {return;right()}",vec!["entry","head","left","tail","drop Tail","drop Left","drop Head"],false),
        ("head(),first:left(),do {return;tail()},second:right()",vec!["entry","head","left","drop Left","drop Head"],false),
        ("head(),first:left(),tail(),second:do {assumes false;right()}",vec!["entry","head","left","tail","drop Tail","drop Left","drop Head"],true),
        ("first:do {return;left()},head(),tail(),second:right()",vec!["entry"],false),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Head {{}} struct Tail {{}} struct Left {{}} struct Right {{}} impl Retire for Head {{fn drop(*self) {{}}}} impl Retire for Tail {{fn drop(*self) {{}}}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn head()->Head {{Head {{}}}} fn tail()->Tail {{Tail {{}}}} fn left()->Left {{Left {{}}}} fn right()->Right {{Right {{}}}} fn take(head:Head,...properties:T,tail:Tail) where T {{}} fn entry() {{take {{{inputs}}};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{inputs}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{inputs}");
    }
}

#[test]
fn tuple_and_property_collectors_cleanup_interleaved_partial_inputs() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (inputs,expected,rejected) in [
        ("head(),first:left(),tail(),second:right()",vec!["entry","head","left","tail","right","take","drop Right","drop Left","drop Tail","drop Head"],false),
        ("head(),first:left(),tail(),second:do {return;right()}",vec!["entry","head","left","tail","drop Tail","drop Left","drop Head"],false),
        ("head(),first:left(),do {return;tail()},second:right()",vec!["entry","head","left","drop Left","drop Head"],false),
        ("head(),first:left(),tail(),second:do {assumes false;right()}",vec!["entry","head","left","tail","drop Tail","drop Left","drop Head"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Head {{}} struct Tail {{}} struct Left {{}} struct Right {{}} impl Retire for Head {{fn drop(*self) {{}}}} impl Retire for Tail {{fn drop(*self) {{}}}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} fn head()->Head {{Head {{}}}} fn tail()->Tail {{Tail {{}}}} fn left()->Left {{Left {{}}}} fn right()->Right {{Right {{}}}} fn take(...items:(Head,Tail),...properties:T) where T {{}} fn entry() {{take {{{inputs}}};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{inputs}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{inputs}");
    }
}

#[test]
fn inherent_generic_methods_cleanup_receiver_and_partial_fields() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (second,expected,rejected) in [
        ("Right {}",vec!["entry","make","take","drop Right","drop Left","drop Holder"],false),
        ("do {return;Right {}}",vec!["entry","make","drop Left","drop Holder"],false),
        ("do {assumes false;Right {}}",vec!["entry","make","drop Left","drop Holder"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Holder {{}} struct Left {{}} struct Right {{}} impl Retire for Holder {{fn drop(*self) {{}}}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{first:Left {{}},second:Right {{}}}}; typealias Options=SAMPLE'type; impl Holder {{fn take(self,...properties:T) where T {{}}}} fn make()->Holder {{Holder {{}}}} fn entry() {{make().take {{first:Left {{}},second:{second}}};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{second}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{second}");
    }
}


#[test]
fn explicit_generic_methods_cleanup_receiver_and_partial_fields() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (second,expected,rejected) in [
        ("Right {}",vec!["entry","make","take","drop Right","drop Left","drop Holder"],false),
        ("do {return;Right {}}",vec!["entry","make","drop Left","drop Holder"],false),
        ("do {assumes false;Right {}}",vec!["entry","make","drop Left","drop Holder"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Holder {{}} struct Left {{}} struct Right {{}} impl Retire for Holder {{fn drop(*self) {{}}}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{first:Left {{}},second:Right {{}}}}; typealias Options=SAMPLE'type; impl Holder {{fn take(self,...properties:T) where T {{}}}} fn make()->Holder {{Holder {{}}}} fn entry() {{make().take<Options> {{first:Left {{}},second:{second}}};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{second}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{second}");
    }
}


#[test]
fn generic_trait_methods_cleanup_receiver_and_partial_fields() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (second,expected,rejected) in [
        ("Right {}",vec!["entry","make","take","drop Right","drop Left","drop Holder"],false),
        ("do {return;Right {}}",vec!["entry","make","drop Left","drop Holder"],false),
        ("do {assumes false;Right {}}",vec!["entry","make","drop Left","drop Holder"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Holder {{}} struct Left {{}} struct Right {{}} impl Retire for Holder {{fn drop(*self) {{}}}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{first:Left {{}},second:Right {{}}}}; typealias Options=SAMPLE'type; trait Take {{fn take(self,...properties:T) where T;}} impl Take for Holder {{fn take(self,...properties:U) where U {{}}}} fn make()->Holder {{Holder {{}}}} fn entry() {{make().as(Take).take {{first:Left {{}},second:{second}}};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{second}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{second}");
    }
}



#[test]
fn explicit_generic_trait_methods_cleanup_receiver_and_partial_fields() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (second,expected,rejected) in [
        ("Right {}",vec!["entry","make","take","drop Right","drop Left","drop Holder"],false),
        ("do {return;Right {}}",vec!["entry","make","drop Left","drop Holder"],false),
        ("do {assumes false;Right {}}",vec!["entry","make","drop Left","drop Holder"],true),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Holder {{}} struct Left {{}} struct Right {{}} impl Retire for Holder {{fn drop(*self) {{}}}} impl Retire for Left {{fn drop(*self) {{}}}} impl Retire for Right {{fn drop(*self) {{}}}} comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{first:Left {{}},second:Right {{}}}}; typealias Options=SAMPLE'type; trait Take {{fn take(self,...properties:T) where T;}} impl Take for Holder {{fn take(self,...properties:U) where U {{}}}} fn make()->Holder {{Holder {{}}}} fn entry() {{make().as(Take).take<Options> {{first:Left {{}},second:{second}}};}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{second}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{second}");
    }
}



#[test]
fn joint_spread_inputs_preserve_affine_owner_cleanup_and_early_exit_order() {
    use comptime_interpretor::{Interpreter,Value};
    for (parameters,input,expected) in [
        ("owner:T,number:.u8'builtin,value:T.as(Source).Element", "make(),...do {(7,true)}", vec!["entry","make","apply","drop Owner"]),
        ("owner:T,number:.u8'builtin,value:T.as(Source).Element", "make(),...do {return;(7,true)}", vec!["entry","make","drop Owner"]),
        ("number:.u8'builtin,value:T.as(Source).Element,owner:T", "...do {return;(7,true)},make()", vec!["entry"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} trait Source {{assoc Element:.Type'builtin=bool;}} struct Owner {{}} impl Source for Owner {{}} impl Retire for Owner {{fn drop(*self) {{}}}} fn make()->Owner {{Owner {{}}}} fn apply({parameters}) where T :- Source {{}} fn entry() {{apply({input});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{input}");
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{input}");
    }
}

#[test]
fn spread_callable_leaves_survive_packet_evaluation_and_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    for packet in [
        "do {(|value| not value,false)}",
        "do {let action:fn(bool)->bool=|value| not value;(action,false)}",
        "if choose() {(negate,false)} else {(identity,true)}",
        "do {let (action,)=(negate,);(action,false)}",
        "Pack {pair:(negate,false),other:identity}.pair",
        "do {let packet=(|value:bool| not value,false);packet}",
        "do {let (packet,)=((negate,false),);packet}",
        "do {let pack=Pack {pair:(identity,false),other:identity};pack.pair=(negate,false);pack.pair}",
        "do {let pack=Pack {pair:(identity,false),other:identity};pack.other=negate;(pack.other,false)}",
        "do {let pack=Pack {pair:(negate,false),other:negate};pack.other=identity;(pack.other,true)}",
        "do {let pack=Pack {pair:(negate,false),other:negate};pack.other=identity;pack.pair}",
        "do {var pack=Pack {pair:(identity,false),other:identity};pack=Pack {pair:(negate,false),other:identity};pack.pair}",
        "do {let pack=Pack {pair:(identity,false),other:identity};if choose() {pack.other=negate;} else {pack.other=identity;}(pack.other,false)}",
        "do {let pack=Pack {pair:(identity,false),other:identity};var again=true;while again {pack.other=negate;again=false;}(pack.other,false)}",
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Owner {{}} impl Retire for Owner {{fn drop(*self) {{}}}} fn make()->Owner {{Owner {{}}}} struct Pack {{pair:(fn(bool)->bool,bool),other:fn(bool)->bool}} fn choose()->bool {{true}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn apply(owner:Owner,action:fn(bool)->bool,value:bool) {{let result=action(value);assumes result;}} fn entry() {{apply(make(),...{packet});}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{packet}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{packet}");
        let names=executed_names(&package,&executable,vm.events());
        let expected=if packet.contains("choose()") {vec!["entry","make","choose","apply","negate","drop Owner"]} else if packet.contains("(pack.other,true)") {vec!["entry","make","apply","identity","drop Owner"]} else if packet.contains("negate") {vec!["entry","make","apply","negate","drop Owner"]} else {vec!["entry","make","apply","drop Owner"]};
        assert_eq!(names,expected,"{packet}");
    }
}

#[test]
fn joint_spread_callable_inputs_preserve_affine_cleanup_and_early_exit_order() {
    use comptime_interpretor::{Interpreter,Value};
    for (parameters,input,expected) in [
        ("owner:T,action:fn(T.as(Source).Element)->T.as(Source).Element,value:T.as(Source).Element", "make(),...do {(|value| not value,false)}", vec!["entry","make","apply","drop Owner"]),
        ("owner:T,action:fn(T.as(Source).Element)->T.as(Source).Element,value:T.as(Source).Element", "make(),...do {return;(|value| not value,false)}", vec!["entry","make","drop Owner"]),
        ("action:fn(T.as(Source).Element)->T.as(Source).Element,value:T.as(Source).Element,owner:T", "...do {return;(|value| not value,false)},make()", vec!["entry"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} trait Source {{assoc Element:.Type'builtin=bool;}} struct Owner {{}} impl Source for Owner {{}} impl Retire for Owner {{fn drop(*self) {{}}}} fn make()->Owner {{Owner {{}}}} fn apply({parameters}) where T :- Source {{}} fn entry() {{apply({input});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{input}");
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{input}");
    }
}

#[test]
fn pattern_callable_leaves_keep_nested_capture_evidence() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} fn entry() {let (action,value)=do {let inner:fn(bool)->bool=|value| not value;(|value:bool| inner(value),false)};let result=action(value);assumes result;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
}

#[test]
fn stored_callable_fields_invoke_replacements_directly() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Operations {action:fn(bool)->bool} fn negate(value:bool)->bool {not value} fn identity(value:bool)->bool {value} fn entry() {let ops=Operations {action:negate};let first=ops.action(false);assumes first;ops.action=identity;let second=ops.action(true);assumes second;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","negate","identity"]);
}

#[test]
fn aggregate_callback_captures_preserve_fields_and_writeback() {
    use comptime_interpretor::{Interpreter,Value};
    for (body,expected) in [
        ("let ops=Operations {action:negate};let callback=|value:bool| {let result=ops.action(value);ops.action=identity;result};let first=callback(false);assumes first;let second=callback(true);assumes second;",vec!["entry","negate","identity"]),
        ("let ops=Operations {action:negate};let callback=|value:bool| ops.action(value);let result=invoke(callback,false);assumes result;",vec!["entry","invoke","negate"]),
        ("let callback=make();let result=invoke(callback,false);assumes result;",vec!["entry","make","invoke","negate"]),
        ("let ops=Operations {action:negate};let callback=|value:bool| ops.action(value);let outer=|value:bool| callback(value);let result=outer(false);assumes result;",vec!["entry","negate"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Operations {{action:fn(bool)->bool}} typealias Callback=%{{ops:Operations}} fn(bool)->bool; fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn invoke(callback:Callback,value:bool)->bool {{callback(value)}} fn make()->Callback {{let ops=Operations {{action:negate}};|value:bool| ops.action(value)}} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn aggregate_callback_capture_retires_affine_environment_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Operations {action:fn(bool)->bool} impl Retire for Operations {fn drop(*self) {}} fn negate(value:bool)->bool {not value} fn entry() {let ops=Operations {action:negate};let callback=|| ops.action(false);let result=callback();assumes result;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","negate","drop Operations"]);
}

#[test]
fn aggregate_function_results_preserve_callable_fields() {
    use comptime_interpretor::{Interpreter,Value};
    for (factory,expected) in [
        ("fn make()->Operations {Operations {action:negate,value:false}}",vec!["entry","make","negate"]),
        ("fn helper()->Operations {Operations {action:negate,value:false}} fn make()->Operations {return helper();}",vec!["entry","make","helper","negate"]),
        ("fn select(flag:bool)->Operations {if flag {return Operations {action:negate,value:false};}Operations {action:identity,value:true}} fn make()->Operations {select(true)}",vec!["entry","make","select","negate"]),
        ("fn pack(action:fn(bool)->bool)->Operations {Operations {action:action,value:false}} fn make()->Operations {pack(negate)}",vec!["entry","make","pack","negate"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Operations {{action:fn(bool)->bool,value:bool}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} {factory} fn entry() {{let ops=make();let result=ops.action(ops.value);assumes result;}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{factory}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{factory}");
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{factory}");
    }
}

#[test]
fn returned_tuple_retains_captured_callback_and_owner_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Owner {} impl Retire for Owner {fn drop(*self) {}} typealias Callback=%{helper:fn(bool)->bool} fn(bool)->bool; fn negate(value:bool)->bool {not value} fn make()->(Callback,bool,Owner) {let helper=negate;(|value:bool| helper(value),false,Owner {})} fn entry() {let (action,value,owner)=make();let result=action(value);assumes result;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","make","negate","drop Owner"]);
}

#[test]
fn aggregate_callable_parameters_cross_function_and_closure_boundaries() {
    use comptime_interpretor::{Interpreter,Value};
    for (declarations,body,expected) in [
        ("fn apply(ops:Operations)->bool {ops.action(ops.value)}", "let result=apply(Operations {action:negate,value:false});assumes result;", vec!["entry","apply","negate"]),
        ("fn forward(ops:Operations)->Operations {ops} fn apply(ops:Operations)->bool {ops.action(ops.value)}", "let result=apply(forward(Operations {action:negate,value:false}));assumes result;", vec!["entry","forward","apply","negate"]),
        ("fn apply(ops:Operations,.enabled:bool=true)->bool {ops.action(ops.value)}", "let result=apply(Operations {action:negate,value:false});assumes result;", vec!["entry","apply","negate"]),
        ("fn apply(.ops:Operations=do {Operations {action:negate,value:false}})->bool {ops.action(ops.value)}", "let result=apply();assumes result;", vec!["entry","apply","negate"]),
        ("fn forward(value:T)->T where T {value} fn apply(ops:Operations)->bool {ops.action(ops.value)}", "let result=apply(forward(Operations {action:negate,value:false}));assumes result;", vec!["entry","forward","apply","negate"]),
        ("", "let apply=|ops:Operations| ops.action(ops.value);let result=apply(Operations {action:negate,value:false});assumes result;", vec!["entry","negate"]),
        ("fn apply(packet:(fn(bool)->bool,bool))->bool {let (action,value)=packet;action(value)}", "let result=apply((negate,false));assumes result;", vec!["entry","apply","negate"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Operations {{action:fn(bool)->bool,value:bool}} fn negate(value:bool)->bool {{not value}} {declarations} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn aggregate_callback_arguments_retire_once_on_success_and_rejection() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (action,rejected) in [("negate",false),("identity",true)] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Operations {{action:fn(bool)->bool}} impl Retire for Operations {{fn drop(*self) {{}}}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn apply(ops:Operations) {{let result=ops.action(false);assumes result;}} fn entry() {{apply(Operations {{action:{action}}});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply",action,"drop Operations"]);
    }
}

#[test]
fn selected_factories_merge_aggregate_results_and_evaluate_arguments_once() {
    use comptime_interpretor::{Interpreter,Value};
    for (choice,selected,callback) in [(true,"first","negate"),(false,"second","identity")] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Operations {{action:fn(bool)->bool,value:bool}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn choose()->bool {{{choice}}} fn marker()->bool {{false}} fn first(value:bool)->Operations {{Operations {{action:negate,value:value}}}} fn second(value:bool)->Operations {{Operations {{action:identity,value:not value}}}} fn entry() {{let factory=if choose() {{first}} else {{second}};let ops=factory(marker());let result=ops.action(ops.value);assumes result;}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose","marker",selected,callback]);
    }
}

#[test]
fn selected_factories_snapshot_callback_before_later_argument_replacement() {
    use comptime_interpretor::{Interpreter,Value};
    for (choice,selected) in [(true,"first"),(false,"second")] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Operations {{action:fn(bool)->bool,value:bool}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn choose()->bool {{{choice}}} fn first(action:fn(bool)->bool,value:bool)->Operations {{Operations {{action:action,value:value}}}} fn second(action:fn(bool)->bool,value:bool)->Operations {{Operations {{action:action,value:value}}}} fn entry() {{let factory=if choose() {{first}} else {{second}};var action:fn(bool)->bool=negate;let ops=factory(action,do {{action=identity;false}});let result=ops.action(ops.value);assumes result;}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose",selected,"negate"]);
    }
}

#[test]
fn selected_factories_validate_pointer_reads_with_callback_inputs() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Operations {action:fn(bool)->bool,value:bool} fn negate(value:bool)->bool {not value} fn choose()->bool {false} fn first(action:fn(bool)->bool,pointer:*bool)->Operations {Operations {action:action,value:pointer.*}} fn second(action:fn(bool)->bool,pointer:*bool)->Operations {Operations {action:action,value:pointer.*}} fn entry() {let factory=if choose() {first} else {second};let value=false;let ops=factory(negate,value.ref);let result=ops.action(ops.value);assumes result;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose","second","negate"]);
}

#[test]
fn pointer_result_calls_keep_callback_parameter_contexts() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} fn negate(value:bool)->bool {not value} fn keep(action:fn(bool)->bool,pointer:*bool)->*bool {pointer} fn entry() {let value=true;let pointer=keep(negate,value.ref);let result=pointer.*;assumes result;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","keep"]);
}

#[test]
fn selected_callback_factories_use_prepared_inputs_for_direct_results() {
    use comptime_interpretor::{Interpreter,Value};
    for (choice,selected) in [(true,"first"),(false,"second")] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn choose()->bool {{{choice}}} fn first(action:fn(bool)->bool,value:bool)->fn(bool)->bool {{action}} fn second(action:fn(bool)->bool,value:bool)->fn(bool)->bool {{action}} fn entry() {{let factory=if choose() {{first}} else {{second}};var action:fn(bool)->bool=negate;let result=factory(action,do {{action=identity;false}});let checked=result(false);assumes checked;}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose",selected,"negate"]);
    }
}

#[test]
fn selected_callback_factories_join_distinct_returned_input_slots() {
    use comptime_interpretor::{Interpreter,Value};
    for (choice,selected,callback,value) in [(true,"first","negate",false),(false,"second","identity",true)] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn choose()->bool {{{choice}}} fn first(a:fn(bool)->bool,b:fn(bool)->bool)->fn(bool)->bool {{a}} fn second(a:fn(bool)->bool,b:fn(bool)->bool)->fn(bool)->bool {{b}} fn entry() {{let factory=if choose() {{first}} else {{second}};let result=factory(negate,identity);let checked=result({value});assumes checked;}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose",selected,callback]);
    }
}

#[test]
fn selected_callback_factories_retain_recursive_worklist_evidence() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} fn negate(value:bool)->bool {not value} fn choose()->bool {true} fn recur(flag:bool)->fn(bool)->bool {if flag {negate} else {recur(true)}} fn direct(flag:bool)->fn(bool)->bool {negate} fn entry() {let factory=if choose() {recur} else {direct};let result=factory(false);let checked=result(false);assumes checked;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose","recur","recur","negate"]);
}

#[test]
fn selected_environment_factories_preserve_actual_captured_values() {
    use comptime_interpretor::{Interpreter,Value};
    for (choice,selected) in [(true,"first"),(false,"second")] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} typealias Callback=%{{value:bool}} fn(bool)->bool; fn choose()->bool {{{choice}}} fn build(flag:bool)->Callback {{let value=flag;|argument:bool| argument and value}} fn first(flag:bool)->Callback {{build(flag)}} fn second(flag:bool)->Callback {{build(not flag)}} fn entry() {{let factory=if choose() {{first}} else {{second}};let callback=factory(true);let result=callback(true);assumes result == {choice};}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose",selected,"build"]);
    }
}

#[test]
fn selected_environment_factories_transfer_affine_captures_through_records() {
    use comptime_interpretor::{Interpreter,Value};
    for (choice,selected) in [(true,"first"),(false,"second")] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Held {{flag:bool}} impl Retire for Held {{fn drop(*self) {{}}}} typealias Callback=%{{held:Held}} fn()->bool; struct Output {{callback:Callback}} fn choose()->bool {{{choice}}} fn build(flag:bool)->Callback {{let held=Held {{flag:flag}};|| held.flag}} fn first(flag:bool)->Output {{Output {{callback:build(flag)}}}} fn second(flag:bool)->Output {{Output {{callback:build(not flag)}}}} fn entry() {{let factory=if choose() {{first}} else {{second}};let output=factory(true);let result=output.callback();assumes result == {choice};}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose",selected,"build","drop Held"]);
    }
}

#[test]
fn computed_capture_initializers_preserve_evaluated_callable_evidence() {
    use comptime_interpretor::{Interpreter,Value};
    use nir::program::{Expr,ExprKind as K};
    for (declarations,body) in [
        ("fn factory()->fn(fn(bool)->bool,bool)->bool {apply}","let helper:fn(fn(bool)->bool,bool)->bool=other_apply;let callback=|| helper(negate,false);let result=callback();assumes result;"),
        ("struct Holder {action:fn(fn(bool)->bool,bool)->bool} fn factory()->Holder {Holder {action:apply}}","let helper:fn(fn(bool)->bool,bool)->bool=other_apply;let callback=|| helper(negate,false);let result=callback();assumes result;"),
        ("struct Operations {action:fn(fn(bool)->bool,bool)->bool} fn factory()->Operations {Operations {action:apply}}","let ops=Operations {action:apply};let callback=|| ops.action(negate,false);let result=callback();assumes result;"),
        ("typealias Carrier=%{helper:fn(fn(bool)->bool,bool)->bool} fn()->bool; fn factory()->Carrier {let helper:fn(fn(bool)->bool,bool)->bool=apply;|| helper(negate,false)}","let helper:fn(fn(bool)->bool,bool)->bool=apply;let inner=|| helper(negate,false);let callback=|| inner();let result=callback();assumes result;"),
    ] {
        let mut package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn negate(value:bool)->bool {{not value}} fn apply(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn other_apply(action:fn(bool)->bool,value:bool)->bool {{not action(value)}} {declarations} fn entry() {{{body}}}"));
        let factory=package.functions.iter().find(|f|f.name=="factory").unwrap();
        let nir::Term::Function {result,..}=factory.signature.callable_signature() else {panic!("factory signature")};
        let result_type=(**result).clone();
        let callee=Expr {source:factory.body.source,ty:factory.signature.clone(),kind:K::Function(factory.declaration)};
        let entry=package.functions.iter_mut().find(|f|f.name=="entry").unwrap();
        let K::Block {statements,..}=&mut entry.body.kind else {panic!("entry block")};
        let closure=statements.iter_mut().rev().find_map(|statement|match &mut statement.kind {K::Let {value,..} if matches!(value.kind,K::Closure {..})=>Some(value),_=>None}).unwrap();
        let K::Closure {captures,..}=&mut closure.kind else {unreachable!()};
        assert_eq!(captures.len(),1);
        let mut initializer=Expr {source:closure.source,ty:result_type,kind:K::Call {callee:Box::new(callee),arguments:vec![],parameter_bindings:vec![]}};
        if initializer.ty != captures[0].ty {
            let nir::Term::Nominal {declaration,..}=initializer.ty.clone() else {panic!("projected holder")};
            initializer=Expr {source:closure.source,ty:captures[0].ty.clone(),kind:K::Field {value:Box::new(initializer),declaration,index:0}};
        }
        captures[0].value=Some(Box::new(initializer));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","factory","apply","negate"]);
    }
}

#[test]
fn enum_payload_callbacks_follow_variant_destructuring() {
    use comptime_interpretor::{Interpreter, Value};
    for body in [
        "let .some(action)=Choice.some(negate) else {return;};let result=action(false);assumes result;",
        "let value=Choice.some(negate);let invoke=|| {let .some(action)=value else {return false;};action(false)};let result=invoke();assumes result;",
        "let value=make();let .some(action)=value else {return;};let result=action(false);assumes result;",
        "let (.some(action),.some(other))=(Choice.some(negate),Choice.some(identity)) else {return;};let first=action(false);let second=other(true);assumes first and second;",
        "let .wrap(.some(action))=Outer.wrap(Choice.some(negate)) else {return;};let result=action(false);assumes result;",
        "let value=if choose() {Choice.some(negate)} else {Choice.some(identity)};let .some(action)=value else {return;};let result=action(false);assumes result;",
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,some(fn(bool)->bool)}} enum Outer {{wrap(Choice)}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn choose()->bool {{true}} fn make()->Choice {{Choice.some(negate)}} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|name|name.as_str()=="negate").count(),1,"{body}: {names:?}");
    }
}

#[test]
fn enum_callback_parameters_use_contextual_cleanup() {
    use comptime_interpretor::{Interpreter, Value};
    for (declarations, body, expected) in [
        ("fn apply(value:Choice)->bool {value match {.none=>false,.some(action)=>action(false)}}", "let result=apply(Choice.some(negate));assumes result;", vec!["entry","apply","negate"]),
        ("fn forward(value:T)->T where T {value} fn apply(value:Choice)->bool {value match {.none=>false,.some(action)=>action(false)}}", "let result=apply(forward(Choice.some(negate)));assumes result;", vec!["entry","forward","apply","negate"]),
        ("fn apply(.value:Choice=do {Choice.some(negate)})->bool {value match {.none=>false,.some(action)=>action(false)}}", "let result=apply();assumes result;", vec!["entry","apply","negate"]),
        ("", "let apply=|value:Choice| value match {.none=>false,.some(action)=>action(false)};let result=apply(Choice.some(negate));assumes result;", vec!["entry","negate"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,some(fn(bool)->bool)}} fn negate(value:bool)->bool {{not value}} {declarations} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn enum_callback_parameters_retire_payload_on_success_and_rejection() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for (action,rejected) in [("negate",false),("identity",true)] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}} enum Choice {{none,some(fn(bool)->bool,Held)}} fn negate(value:bool)->bool {{not value}} fn identity(value:bool)->bool {{value}} fn apply(value:Choice) {{value match {{.none=>{{}},.some(action,held)=>{{let result=action(false);assumes result;}}}};}} fn entry() {{apply(Choice.some({action},Held {{}}));}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Unit);}
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply",action,"drop Held"]);
    }
}

#[test]
fn enum_branch_domains_preserve_guarded_higher_order_callbacks() {
    use comptime_interpretor::{Interpreter,Value};
    for choice in [true,false] {
        for setup in [
            "let value=if choose() {Choice.left(first)} else {Choice.right(second)};",
            "var value=Choice.left(first);if choose() {value=Choice.left(first);} else {value=Choice.right(second);}",
            "let (value,_)=if choose() {(Choice.left(first),false)} else {(Choice.right(second),true)};",
            "let .wrap(value)=if choose() {Outer.wrap(Choice.left(first))} else {Outer.wrap(Choice.right(second))};",
            "var packet=Packet {value:Choice.left(first),flag:false};if choose() {packet.value=Choice.left(first);} else {packet.value=Choice.right(second);}let value=packet.value;",
            "let value=make();",
            "var value=Choice.right(second);var run=choose();while run {value=Choice.left(first);run=false;}",
            "let value=(if choose() {Packet {value:Choice.left(first),flag:false}} else {Packet {value:Choice.right(second),flag:true}}).value;",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{left(fn(fn(bool)->bool,bool)->bool),right(fn(fn(bool)->bool,bool)->bool)}} enum Outer {{wrap(Choice)}} struct Packet {{value:Choice,flag:bool}} fn choose()->bool {{{choice}}} fn negate(value:bool)->bool {{not value}} fn first(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn second(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn make()->Choice {{if choose() {{Choice.left(first)}} else {{Choice.right(second)}}}} fn dispatch(value:Choice)->bool {{value match {{.left(action)=>action(negate,false),.right(action)=>action(negate,false)}}}} fn entry() {{{setup} let result=dispatch(value);assumes result;}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{choice}: {setup}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{choice}: {setup}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="choose").count(),1,"{names:?}");
            assert_eq!(&names[names.len()-3..],["dispatch",if choice {"first"} else {"second"},"negate"],"{setup}");
        }
    }
}

#[test]
fn enum_empty_branch_does_not_require_a_callback_value() {
    use comptime_interpretor::{Interpreter,Value};
    for choice in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Maybe {{none,some(fn(fn(bool)->bool,bool)->bool)}} fn choose()->bool {{{choice}}} fn negate(value:bool)->bool {{not value}} fn apply(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn dispatch(value:Maybe)->bool {{value match {{.none=>false,.some(action)=>action(negate,false)}}}} fn entry() {{let value=if choose() {{Maybe.some(apply)}} else {{Maybe.none}};let result=dispatch(value);assumes result == {choice};}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        let expected=if choice {vec!["entry","choose","dispatch","apply","negate"]} else {vec!["entry","choose","dispatch"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn enum_return_domains_cross_calls_and_distinct_normal_exits() {
    use comptime_interpretor::{Interpreter,Value};
    for choice in [true,false] {
        for setup in [
            "let value=make(choose());",
            "let value=forward(choose());",
            "let value=if choose() {left()} else {right()};",
            "let factory=if choose() {left} else {right};let value=factory();",
            "let factory=if choose() {left_packet} else {right_packet};let value=factory().value;",
            "let value=(if choose() {left_packet()} else {right_packet()}).value;",
            "let factory=|flag:bool| {if flag {return Choice.left(first);}Choice.right(second)};let value=factory(choose());",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{left(fn(fn(bool)->bool,bool)->bool),right(fn(fn(bool)->bool,bool)->bool)}} struct Packet {{value:Choice}} fn choose()->bool {{{choice}}} fn negate(value:bool)->bool {{not value}} fn first(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn second(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn left()->Choice {{Choice.left(first)}} fn right()->Choice {{Choice.right(second)}} fn left_packet()->Packet {{Packet {{value:left()}}}} fn right_packet()->Packet {{Packet {{value:right()}}}} fn make(flag:bool)->Choice {{if flag {{return left();}}right()}} fn forward(flag:bool)->Choice {{make(flag)}} fn dispatch(value:Choice)->bool {{value match {{.left(action)=>action(negate,false),.right(action)=>action(negate,false)}}}} fn entry() {{{setup}let result=dispatch(value);assumes result;}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{choice}: {setup}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{choice}: {setup}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="choose").count(),1,"{names:?}");
            assert_eq!(&names[names.len()-3..],["dispatch",if choice {"first"} else {"second"},"negate"],"{setup}");
        }
    }
}

#[test]
fn enum_return_summaries_keep_empty_and_callback_exits_distinct() {
    use comptime_interpretor::{Interpreter,Value};
    for choice in [true,false] {
        for body in [
            "let value=make(choose());",
            "let factory=if choose() {some} else {none};let value=factory();",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Maybe {{none,some(fn(fn(bool)->bool,bool)->bool)}} fn choose()->bool {{{choice}}} fn negate(value:bool)->bool {{not value}} fn apply(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn some()->Maybe {{Maybe.some(apply)}} fn none()->Maybe {{Maybe.none}} fn make(flag:bool)->Maybe {{if flag {{return some();}}none()}} fn dispatch(value:Maybe)->bool {{value match {{.none=>false,.some(action)=>action(negate,false)}}}} fn entry() {{{body}let result=dispatch(value);assumes result == {choice};}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{choice}: {body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="choose").count(),1);
            assert_eq!(names.iter().filter(|name|name.as_str()=="apply").count(),usize::from(choice));
            assert_eq!(names.iter().filter(|name|name.as_str()=="negate").count(),usize::from(choice));
        }
    }
}

#[test]
fn enum_parameter_domains_discharge_only_inapplicable_variants() {
    use comptime_interpretor::{Interpreter,Value};
    for (input,expected_value,callback) in [("Choice.none",false,None),("Choice.left(first)",true,Some("first")),("Choice.right(second)",true,Some("second"))] {
        for (declarations,body) in [
            ("",format!("let result=dispatch({input});assumes result == {expected_value};")),
            ("fn forward(value:T)->T where T {value}",format!("let result=dispatch(forward({input}));assumes result == {expected_value};")),
            ("fn packed(value:Packet)->bool {dispatch(value.value)}",format!("let result=packed(Packet {{value:{input}}});assumes result == {expected_value};")),
            ("",format!("let invoke=|value:Choice| value match {{.none=>false,.left(action)=>action(negate,false),.right(action)=>action(negate,false)}};let result=invoke({input});assumes result == {expected_value};")),
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,left(fn(fn(bool)->bool,bool)->bool),right(fn(fn(bool)->bool,bool)->bool)}} struct Packet {{value:Choice}} fn negate(value:bool)->bool {{not value}} fn first(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn second(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn dispatch(value:Choice)->bool {{value match {{.none=>false,.left(action)=>action(negate,false),.right(action)=>action(negate,false)}}}} {declarations} fn entry() {{{body}}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="first" || name.as_str()=="second").map(String::as_str).collect::<Vec<_>>(),callback.into_iter().collect::<Vec<_>>(),"{body}");
            assert_eq!(names.iter().filter(|name|name.as_str()=="negate").count(),usize::from(expected_value));
        }
    }
}

#[test]
fn enum_parameter_domains_snapshot_before_later_arguments_and_defaults() {
    use comptime_interpretor::{Interpreter,Value};
    for (declaration,body,expected) in [
        ("fn dispatch(value:Choice,ignored:bool)->bool {value match {.none=>false,.some(action)=>action(negate,false)}}","var value=Choice.none;let result=dispatch(value,do {value=Choice.some(apply);true});assumes not result;let updated=dispatch(value,false);assumes updated;",vec!["entry","dispatch","dispatch","apply","negate"]),
        ("fn dispatch(.value:Choice=Choice.none)->bool {value match {.none=>false,.some(action)=>action(negate,false)}}","let result=dispatch();assumes not result;",vec!["entry","dispatch"]),
        ("fn forward(value:Choice,.ignored:bool=true)->Choice {value} fn dispatch(value:Choice)->bool {value match {.none=>false,.some(action)=>action(negate,false)}}","let result=dispatch(forward(Choice.none));assumes not result;",vec!["entry","forward","dispatch"]),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,some(fn(fn(bool)->bool,bool)->bool)}} fn negate(value:bool)->bool {{not value}} fn apply(action:fn(bool)->bool,value:bool)->bool {{action(value)}} {declaration} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
        assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
    }
}

#[test]
fn enum_parameter_tags_do_not_discharge_pattern_guards() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,some(fn(fn(bool)->bool,bool)->bool)}} fn negate(value:bool)->bool {{not value}} fn apply(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn guarded(value:Choice,flag:bool)->bool {{value match {{.some(action) if flag=>action(negate,false),_=>false}}}} fn entry() {{let result=guarded(Choice.some(apply),{flag});assumes result == {flag};}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        let expected=if flag {vec!["entry","guarded","apply","negate"]} else {vec!["entry","guarded"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn enum_parameter_tags_cross_shared_memory_preparation() {
    use comptime_interpretor::{Interpreter,Value};
    for choice in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,some(fn(bool)->bool)}} fn choose()->bool {{{choice}}} fn first(value:Choice,pointer:*bool)->Choice {{let observed=pointer.*;value}} fn second(value:Choice,pointer:*bool)->Choice {{let observed=pointer.*;value}} fn keep(value:Choice,pointer:*bool)->*bool {{let ignored=value;pointer}} fn entry()->bool {{let factory=if choose() {{first}} else {{second}};let flag=true;let .none=factory(Choice.none,flag.ref) else {{return false;}};let pointer=keep(Choice.none,flag.ref);pointer.*}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","choose",if choice {"first"} else {"second"},"keep"]);
    }
}

#[test]
fn captured_enum_domains_survive_return_and_nested_callables() {
    use comptime_interpretor::{Interpreter,Value};
    for (input,expected,selected) in [("Choice.none",false,None),("Choice.left(first)",true,Some("first")),("Choice.right(second)",true,Some("second"))] {
        for body in [
            format!("let value={input};let callback=|| dispatch(value);let result=callback();assumes result == {expected};"),
            format!("let value={input};let callback=|| dispatch(value);let result=call(callback);assumes result == {expected};"),
            format!("let callback=build({input});let result=call(callback);assumes result == {expected};"),
            format!("let callback=forward(build({input}));let nested=|| callback();let result=nested();assumes result == {expected};"),
            format!("let packet=Packet {{value:{input}}};let callback=|| dispatch(packet.value);let result=callback();assumes result == {expected};"),
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,left(fn(fn(bool)->bool,bool)->bool),right(fn(fn(bool)->bool,bool)->bool)}} struct Packet {{value:Choice}} typealias Callback=%{{value:Choice}} fn()->bool; fn negate(value:bool)->bool {{not value}} fn first(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn second(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn dispatch(value:Choice)->bool {{value match {{.none=>false,.left(action)=>action(negate,false),.right(action)=>action(negate,false)}}}} fn build(value:Choice)->Callback {{|| dispatch(value)}} fn call(callback:Callback)->bool {{callback()}} fn forward(value:T)->T where T {{value}} fn entry() {{{body}}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="first" || name.as_str()=="second").map(String::as_str).collect::<Vec<_>>(),selected.into_iter().collect::<Vec<_>>(),"{body}");
        }
    }
}

#[test]
fn captured_enum_domains_follow_mutation_and_multiple_normal_exits() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} enum Choice {none,left(fn(fn(bool)->bool,bool)->bool),right(fn(fn(bool)->bool,bool)->bool)} fn negate(value:bool)->bool {not value} fn first(action:fn(bool)->bool,value:bool)->bool {action(value)} fn second(action:fn(bool)->bool,value:bool)->bool {action(value)} fn dispatch(value:Choice)->bool {value match {.none=>false,.left(action)=>action(negate,false),.right(action)=>action(negate,false)}} fn entry() {var value=Choice.none;let callback=|flag:bool| {let current=value;if flag {value=Choice.left(first);return dispatch(current);}value=Choice.right(second);dispatch(current)};let empty=callback(true);assumes not empty;let left=callback(false);assumes left;let right=callback(true);assumes right;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","dispatch","dispatch","first","negate","dispatch","second","negate"]);
}

#[test]
fn captured_enum_domains_join_same_code_factory_results() {
    use comptime_interpretor::{Interpreter,Value};
    for choice in [true,false] {
        for setup in ["let factory=if choose() {some} else {none};let callback=factory();","let callback=make(choose());"] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{none,some(fn(fn(bool)->bool,bool)->bool)}} typealias Callback=%{{value:Choice}} fn()->bool; fn choose()->bool {{{choice}}} fn negate(value:bool)->bool {{not value}} fn apply(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn dispatch(value:Choice)->bool {{value match {{.none=>false,.some(action)=>action(negate,false)}}}} fn build(value:Choice)->Callback {{|| dispatch(value)}} fn some()->Callback {{build(Choice.some(apply))}} fn none()->Callback {{build(Choice.none)}} fn make(flag:bool)->Callback {{if flag {{return some();}}none()}} fn call(callback:Callback)->bool {{callback()}} fn entry() {{{setup}let result=call(callback);assumes result == {choice};}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{choice}: {setup}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="apply").count(),usize::from(choice),"{names:?}");
            assert_eq!(names.iter().filter(|name|name.as_str()=="negate").count(),usize::from(choice),"{names:?}");
        }
    }
}

#[test]
fn captured_enum_failure_paths_retire_affine_owners_once() {
    use comptime_interpretor::{ErrorKind,Interpreter};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Held {{}} struct Caller {{}} impl Retire for Held {{fn drop(*self) {{}}}} impl Retire for Caller {{fn drop(*self) {{}}}} enum Choice {{none,some(fn(bool)->bool)}} fn action(value:bool)->bool {{value}} fn reject(value:Choice)->bool {{assumes false;false}} fn entry() {{let caller=Caller {{}};let held=Held {{}};var value=Choice.none;let callback=|flag:bool| {{let owned=held;let current=value;if flag {{value=Choice.some(action);return reject(current);}}value=Choice.none;reject(current)}};let result=callback({flag});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","reject","drop Held","drop Caller"]);
    }
}

#[test]
fn closure_failure_joins_distinct_remaining_capture_owners() {
    use comptime_interpretor::{ErrorKind,Interpreter};
    for flag in [true,false] {
        for (setup,left,right) in [
            ("let first=First {};let second=Second {};","first","second"),
            ("let pair=Pair {first:First {},second:Second {}};","pair.first","pair.second"),
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct First {{}} struct Second {{}} struct Caller {{}} struct Pair {{first:First,second:Second}} impl Retire for First {{fn drop(*self) {{}}}} impl Retire for Second {{fn drop(*self) {{}}}} impl Retire for Caller {{fn drop(*self) {{}}}} fn take_first(value:First) {{}} fn take_second(value:Second) {{}} fn reject() {{assumes false;}} fn entry() {{let caller=Caller {{}};{setup}let callback=|flag:bool| {{if flag {{take_first({left});reject();}} else {{take_second({right});reject();}}}};callback({flag});}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{flag}: {setup}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
            let expected=if flag {vec!["entry","take_first","drop First","reject","drop Second","drop Caller"]} else {vec!["entry","take_second","drop Second","reject","drop First","drop Caller"]};
            assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{flag}: {setup}");
        }
    }
}

#[test]
fn conditional_cleanup_plans_preserve_independent_parent_and_field_tokens() {
    use elaboration::verification::{drop::{plan_cleanup, CleanupStep}, ownership::{Place, Shape, State}, PlaceId};
    use std::collections::BTreeSet;
    let package = compile("trait Retire {fn drop(*self);} struct Child {} struct Parent {first:Child,second:Child} impl Retire for Child {fn drop(*self) {}} impl Retire for Parent {fn drop(*self) {}}");
    let root = PlaceId {owner:1,local:1};
    let receiver = Place {root,fields:vec![]};
    let concrete = |constructed:bool, mask:u8| {
        let mut state = State::default();
        state.declare(root, &Shape::Fields(vec![Shape::Fields(vec![]);2]), constructed, &mut 100).unwrap();
        let mut tokens = BTreeSet::new();
        if constructed {tokens.insert(vec![]);}
        for index in 0..2 {
            let place = Place {root, fields:vec![index]};
            if mask & (1 << index) != 0 {
                tokens.insert(vec![index]);
                if !constructed {state.install(&place).unwrap();}
            } else if constructed {state.take(&place).unwrap();}
        }
        (state, tokens)
    };
    fn trace(steps:&[CleanupStep], tokens:&mut BTreeSet<Vec<usize>>, output:&mut Vec<Vec<usize>>) {
        for step in steps {
            match step {
                CleanupStep::IfConstructed {place,steps} => {
                    if tokens.contains(&place.fields) {trace(steps,tokens,output);}
                }
                CleanupStep::Invoke {access,..} => {
                    assert!(tokens.contains(&access.receiver.fields));
                    output.push(access.receiver.fields.clone());
                }
                CleanupStep::Release {place,..} => {assert!(tokens.remove(&place.fields));}
                CleanupStep::SwitchVariant {..} | CleanupStep::SelectVariantPlan {..} => panic!("non-enum layout"),
            }
        }
    }
    let cases:Vec<_> = [false,true].into_iter().flat_map(|constructed|
        (0..4).filter(move |mask| constructed || *mask != 3).map(move |mask|(constructed,mask))
    ).collect();
    world(&package, |world,items| {
        let acquired = luna::protocols::acquire_cleanup(&package,world,items,&nominal(&package,"Parent",vec![]),Budget::default(),&mut 10000).unwrap();
        for &(lc,lm) in &cases {for &(rc,rm) in &cases {
            let (left,lt) = concrete(lc,lm);
            let (right,rt) = concrete(rc,rm);
            let joined = left.join(&right,&mut 100).unwrap();
            let before = joined.clone();
            let plan = plan_cleanup(&package,world,&acquired,&joined,&receiver,&mut 10000).unwrap();
            assert_eq!(joined,before);
            for mut tokens in [lt,rt] {
                let expected:Vec<_> = [vec![],vec![1],vec![0]].into_iter().filter(|p|tokens.contains(p)).collect();
                let mut actual = vec![];
                trace(&plan.steps,&mut tokens,&mut actual);
                assert_eq!(actual,expected,"{lc}/{lm}, {rc}/{rm}: {plan}");
                assert!(tokens.is_empty());
            }
        }}
        let (full,_) = concrete(true,3);
        let (empty,_) = concrete(false,0);
        let conditional = full.join(&empty,&mut 100).unwrap();
        assert!(plan_cleanup(&package,world,&acquired,&conditional,&receiver,&mut 0).is_err());
        let plan = plan_cleanup(&package,world,&acquired,&conditional,&receiver,&mut 10000).unwrap();
        assert!(plan.to_string().contains("(if-constructed (owned-place"));
        println!("CONDITIONAL_CLEANUP_DUMP={plan}");
        let prepared = luna::cleanup::prepare(&package,world,&acquired,&conditional,&receiver,&[],&mut 10000,Budget::default()).unwrap();
        assert!(prepared.program.to_string().contains("(cleanup-if-constructed"));
    });
}

#[test]
fn conditional_enum_cleanup_guards_tag_reads_and_keeps_variant_evidence() {
    use elaboration::verification::{drop::{plan_cleanup,plan_cleanup_with_variants,CleanupStep}, moves::VariantSelection, ownership::{Place,Shape,State}, PlaceId};
    let package = compile("trait Retire {fn drop(*self);} struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} impl Retire for Ticket {fn drop(*self) {}} impl Retire for Choice {fn drop(*self) {}}");
    world(&package, |world,items| {
        let ty = nir::Term::Nominal {declaration:package.enums[0].declaration,arguments:vec![]};
        let acquired = luna::protocols::acquire_cleanup(&package,world,items,&ty,Budget::default(),&mut 10000).unwrap();
        let receiver = Place {root:PlaceId {owner:1,local:0},fields:vec![]};
        let mut present = State::default();
        present.declare(receiver.root,&Shape::Value,true,&mut 100).unwrap();
        let mut absent = present.clone();absent.take(&receiver).unwrap();
        let joined = present.join(&absent,&mut 100).unwrap();
        let plan = plan_cleanup(&package,world,&acquired,&joined,&receiver,&mut 10000).unwrap();
        assert_eq!(plan.steps.len(),3,"{plan}");
        for step in &plan.steps {assert!(matches!(step,CleanupStep::IfConstructed {place,..} if place==&receiver));}
        let CleanupStep::IfConstructed {steps,..} = &plan.steps[1] else {unreachable!()};
        let CleanupStep::SwitchVariant {branches,..} = &steps[0] else {panic!("{plan}")};
        assert!(branches[0].steps.is_empty());
        let paths:Vec<_> = branches[1].steps.iter().filter_map(|step|match step {
            CleanupStep::Invoke {access,..} => Some(access.receiver.fields.clone()),_=>None,
        }).collect();
        assert_eq!(paths,[vec![1],vec![0]]);
        let known = plan_cleanup_with_variants(&package,world,&acquired,&joined,&receiver,&[VariantSelection {fields:vec![],index:1}],&mut 10000).unwrap();
        let CleanupStep::IfConstructed {steps,..} = &known.steps[1] else {unreachable!()};
        assert!(!steps.iter().any(|s|matches!(s,CleanupStep::SwitchVariant {..})));
        assert_eq!(steps.len(),4);
        assert_eq!(joined.require_present(&receiver),Err(elaboration::verification::ownership::Error::NotPresent));
    });
}

#[test]
fn closure_failure_analysis_joins_remaining_captures_once_at_the_call_boundary() {
    use elaboration::verification::moves::{CleanupReason,CleanupAction};
    for (setup,left,right) in [
        ("let first=First {};let second=Second {};","first","second"),
        ("let pair=Pair {first:First {},second:Second {}};","pair.first","pair.second"),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct First {{}} struct Second {{}} struct Caller {{}} struct Pair {{first:First,second:Second}} impl Retire for First {{fn drop(*self) {{}}}} impl Retire for Second {{fn drop(*self) {{}}}} impl Retire for Caller {{fn drop(*self) {{}}}} fn take_first(value:First) {{}} fn take_second(value:Second) {{}} fn reject() {{assumes false;}} fn entry(flag:bool) {{let caller=Caller {{}};{setup}let callback=|flag:bool| {{if flag {{take_first({left});reject();}} else {{take_second({right});reject();}}}};callback(flag);}}"));
        let report=cleanup_report(&package,"entry").unwrap();
        let failures:Vec<_>=report.cleanup_sites.iter().filter(|site|site.reason==CleanupReason::PredicateFailure).collect();
        assert_eq!(failures.len(),1,"{report}");
        assert!(failures[0].actions.iter().any(|action|match action {
            CleanupAction::Planned {plan,..} => plan.to_string().contains("(if-constructed"), _=>false,
        }),"{report}");
        assert!(report.closures.iter().any(|closure|closure.invoked && closure.invocation.failed_exits.len()==2));
    }
}

#[test]
fn runtime_conditional_cleanup_handles_owned_roots_and_opaque_enum_payloads() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for (declarations,setup,ty,drops) in [
            ("", "let value=Ticket {};", "Ticket", 1),
            ("enum Choice {empty,pair(Ticket,Ticket)}", "let value=Choice.pair(Ticket {},Ticket {});", "Choice", 2),
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ticket {{}} {declarations} impl Retire for Ticket {{fn drop(*self) {{}}}} fn take(value:{ty}) {{}} fn work(flag:bool) {{{setup}if flag {{take(value);}}}} fn entry() {{work({flag});}}"));
            let executable=source_executable(&package,"entry").unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
            let mut expected=vec!["entry","work"];
            if flag {expected.push("take");}
            expected.extend(std::iter::repeat_n("drop Ticket",drops));
            assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{ty}/{flag}");
            assert!(executable.schedule.to_string().contains("cleanup-if-constructed"));
            if !flag && ty == "Choice" { println!("RUNTIME_CONSTRUCTION_DUMP={}", executable.schedule); }
        }
    }
}

#[test]
fn runtime_conditional_replacement_reinstalls_captured_fields_and_whole_owners() {
    use comptime_interpretor::{Interpreter,Value};
    for (setup,place) in [
        ("var value=Ticket {};","value"),
        ("let pair=Pair {value:Ticket {}};","pair.value"),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ticket {{}} struct Pair {{value:Ticket}} impl Retire for Ticket {{fn drop(*self) {{}}}} fn take(value:Ticket) {{}} fn entry() {{{setup}let callback=|flag:bool| {{if flag {{take({place});}}{place}=Ticket {{}};}};callback(true);callback(false);}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Ticket","drop Ticket","drop Ticket"],"{place}");
    }
}

#[test]
fn runtime_field_reconstruction_restores_parent_responsibility_only_when_complete() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} struct Pair {first:Ticket,second:Ticket} impl Retire for Ticket {fn drop(*self) {}} fn take(value:Pair) {} fn entry() {let pair=Pair {first:Ticket {},second:Ticket {}};take(pair);pair.first=Ticket {};pair.second=Ticket {};take(pair);}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","take","drop Ticket","drop Ticket","take","drop Ticket","drop Ticket"]);
}

#[test]
fn cleanup_schedule_rejects_duplicate_transfer_hooks_before_execution() {
    use comptime_interpretor::{ErrorKind,Interpreter};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Ticket {} impl Retire for Ticket {fn drop(*self) {}} fn take(value:Ticket) {} fn entry() {let value=Ticket {};take(value);}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut schedule=executable.schedule.clone();
    let scope=schedule.functions.iter_mut().find(|scope|!scope.transfers.is_empty()).unwrap();
    scope.transfers.push(scope.transfers[0].clone());
    let error=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&schedule).err().unwrap();
    assert_eq!(error.kind,ErrorKind::InvalidCallPlan);
}

#[test]
fn conditionally_escaped_closure_keeps_capture_responsibility_with_its_new_owner() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}} typealias Callback = %{{ticket:Ticket}} fn(); fn build()->Callback {{let ticket=Ticket {{}};|| {{let pointer=ticket.ref;}}}} fn forward(value:Callback)->Callback {{value}} fn work(flag:bool) {{let saved=do {{let original=build();if flag {{forward(original)}} else {{build()}}}};saved();}} fn entry() {{work({flag});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        let expected=if flag {vec!["entry","work","build","forward","drop Ticket"]}else{vec!["entry","work","build","build","drop Ticket","drop Ticket"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn conditional_drop_field_access_uses_construction_implications_across_closure_frames() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let cell=Cell {value:true};if flag {take(cell);}",
            "let cell=Cell {value:true};let callback=|flag:bool| {if flag {take(cell);}};callback(flag);",
            "let pair=Pair {cell:Cell {value:true}};let callback=|flag:bool| {if flag {take(pair.cell);}};callback(flag);",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Cell {{value:bool}} struct Pair {{cell:Cell}} impl Retire for Cell {{fn drop(*self) {{let seen=self.value.*;}}}} fn take(value:Cell) {{}} fn work(flag:bool) {{{body}}} fn entry() {{work({flag});}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{body}: {e}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
            let expected=if flag {vec!["entry","work","take","drop Cell"]}else{vec!["entry","work","drop Cell"]};
            assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
        }
    }
}

#[test]
fn partial_reconstruction_runs_parent_drop_only_on_paths_completed_by_the_last_write() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ticket {{}} struct Pair {{first:Ticket,second:Ticket}} impl Retire for Ticket {{fn drop(*self) {{}}}} impl Retire for Pair {{fn drop(*self) {{}}}} fn take(value:Pair) {{}} fn work(flag:bool) {{let pair=Pair {{first:Ticket {{}},second:Ticket {{}}}};take(pair);if flag {{pair.first=Ticket {{}};}}pair.second=Ticket {{}};}} fn entry() {{work({flag});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        let mut expected=vec!["entry","work","take","drop Pair","drop Ticket","drop Ticket"];
        if flag {expected.extend(["drop Pair","drop Ticket","drop Ticket"]);} else {expected.push("drop Ticket");}
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn construction_implications_survive_failed_closure_capture_writeback() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    for flag in [true,false] {
        for (setup,place) in [
            ("let cell=Cell {value:true};","cell"),
            ("let pair=Pair {cell:Cell {value:true}};","pair.cell"),
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Cell {{value:bool}} struct Pair {{cell:Cell}} struct Caller {{}} impl Retire for Caller {{fn drop(*self) {{}}}} impl Retire for Cell {{fn drop(*self) {{let seen=self.value.*;}}}} fn take(value:Cell) {{}} fn reject() {{assumes false;}} fn work(flag:bool) {{let caller=Caller {{}};{setup}let callback=|flag:bool| {{if flag {{take({place});}}reject();}};callback(flag);}} fn entry() {{work({flag});}}"));
            let executable=source_executable(&package,"entry").unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
            let expected=if flag {vec!["entry","work","take","drop Cell","reject","drop Caller"]}else{vec!["entry","work","reject","drop Cell","drop Caller"]};
            assert_eq!(executed_names(&package,&executable,vm.events()),expected);
        }
    }
}

#[test]
fn reconstructed_parent_guards_allow_reads_only_of_proved_live_fields() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for (body,expected) in [
            ("let pair=Pair {first:Ticket {value:true},second:Ticket {value:true}};if flag {take(pair);}pair.first=Ticket {value:true};",
                if flag {vec!["entry","work","take","drop Pair","drop Ticket","drop Ticket","drop Ticket"]}else{vec!["entry","work","drop Ticket","drop Pair","drop Ticket","drop Ticket"]}),
            ("let pair=Pair {first:Ticket {value:true},second:Ticket {value:true}};take(pair);if flag {pair.first=Ticket {value:true};}pair.second=Ticket {value:true};",
                if flag {vec!["entry","work","take","drop Pair","drop Ticket","drop Ticket","drop Pair","drop Ticket","drop Ticket"]}else{vec!["entry","work","take","drop Pair","drop Ticket","drop Ticket","drop Ticket"]}),
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ticket {{value:bool}} struct Pair {{first:Ticket,second:Ticket}} impl Retire for Ticket {{fn drop(*self) {{let seen=self.value.*;}}}} impl Retire for Pair {{fn drop(*self) {{let first=self.first.value.*;let second=self.second.value.*;}}}} fn take(value:Pair) {{}} fn work(flag:bool) {{{body}}} fn entry() {{work({flag});}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{body}: {e}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
            assert_eq!(executed_names(&package,&executable,vm.events()),expected,"{body}");
        }
    }
}

#[test]
fn conditional_decomposed_enum_cleanup_preserves_payloads_outside_the_parent_guard() {
    use elaboration::verification::{drop::{plan_cleanup_with_variants,CleanupStep},moves::VariantSelection,ownership::{State,Shape,Place},PlaceId};
    use comptime_interpretor::{Interpreter,Storage,Value,Event,ErrorKind};
    use std::collections::BTreeSet;
    let package=compile("trait Retire {fn drop(*self);} struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} impl Retire for Ticket {fn drop(*self) {}} impl Retire for Choice {fn drop(*self) {}}");
    let root=PlaceId {owner:23,local:0};let receiver=Place {root,fields:vec![]};
    let ty=nir::Term::Nominal {declaration:package.enums[0].declaration,arguments:vec![]};
    let evidence=[VariantSelection {fields:vec![],index:1}];
    let concrete=|constructed:bool,mask:u8| {
        let mut state=State::default();
        state.declare(root,&Shape::Fields(vec![Shape::Fields(vec![]);2]),constructed,&mut 100).unwrap();
        let mut tokens=BTreeSet::new();if constructed {tokens.insert(vec![]);}
        for index in 0..2 {
            let field=Place {root,fields:vec![index]};
            if mask & (1<<index)!=0 {
                tokens.insert(vec![index]);if !constructed {state.install(&field).unwrap();}
            } else if constructed {state.take(&field).unwrap();}
        }
        (state,tokens)
    };
    fn trace(steps:&[CleanupStep],tokens:&mut BTreeSet<Vec<usize>>,calls:&mut Vec<Vec<usize>>) {
        for step in steps {match step {
            CleanupStep::IfConstructed {place,steps} => {if tokens.contains(&place.fields) {trace(steps,tokens,calls);}}
            CleanupStep::Invoke {access,..} => {assert!(tokens.contains(&access.receiver.fields));calls.push(access.receiver.fields.clone());}
            CleanupStep::Release {place,..} => {assert!(tokens.remove(&place.fields));}
            CleanupStep::SwitchVariant {..} | CleanupStep::SelectVariantPlan {..} => panic!("selected payload must not dispatch other variants"),
        }}
    }
    let cases:Vec<_>=[false,true].into_iter().flat_map(|constructed|(0u8..4).filter(move|mask|constructed || *mask!=3).map(move|mask|(constructed,mask))).collect();
    world(&package,|world,items| {
        let acquired=luna::protocols::acquire_cleanup(&package,world,items,&ty,Budget::default(),&mut 10000).unwrap();
        for &(lc,lm) in &cases {for &(rc,rm) in &cases {
            let (left,lt)=concrete(lc,lm);let (right,rt)=concrete(rc,rm);
            let joined=left.join(&right,&mut 1000).unwrap();
            let plan=plan_cleanup_with_variants(&package,world,&acquired,&joined,&receiver,&evidence,&mut 10000).unwrap();
            for mut tokens in [lt,rt] {
                let expected:Vec<_>=[vec![],vec![1],vec![0]].into_iter().filter(|path|tokens.contains(path)).collect();
                let mut actual=vec![];trace(&plan.steps,&mut tokens,&mut actual);
                assert_eq!(actual,expected,"{lc}/{lm}, {rc}/{rm}: {plan}");assert!(tokens.is_empty());
            }
        }}
        let (full,_)=concrete(true,3);let (partial,_)=concrete(false,1);
        let joined=full.join(&partial,&mut 1000).unwrap();let snapshot=joined.clone();
        assert!(plan_cleanup_with_variants(&package,world,&acquired,&joined,&receiver,&[],&mut 10000).unwrap_err().contains("active variant"));
        for index in [0,99] {assert!(plan_cleanup_with_variants(&package,world,&acquired,&joined,&receiver,&[VariantSelection {fields:vec![],index}],&mut 10000).is_err());}
        assert!(plan_cleanup_with_variants(&package,world,&acquired,&joined,&receiver,&evidence,&mut 0).is_err());
        assert_eq!(joined,snapshot);
        let prepared=luna::cleanup::prepare(&package,world,&acquired,&joined,&receiver,&evidence,&mut 100000,Budget::default()).unwrap();
        println!("DECOMPOSED_ENUM_CLEANUP_DUMP={}",prepared.program);
        let ticket=Value::Record {declaration:package.structs[0].declaration,arguments:vec![],fields:vec![]};
        for (index,fields) in [(0,vec![]),(1,vec![ticket.clone(),ticket])] {
            let value=Value::Variant {declaration:package.enums[0].declaration,arguments:vec![],index,fields};
            let storage=Storage::new(value.clone());
            let mut vm=Interpreter::from_iir(&prepared.graph,Default::default()).unwrap();
            let result=vm.execute_cleanup(&prepared.program,&storage);
            if index==0 {
                assert_eq!(result.unwrap_err().kind,ErrorKind::InvalidCallPlan);
                assert!(vm.events().is_empty());assert_eq!(storage.read().unwrap(),value);
            } else {
                result.unwrap();assert_eq!(storage.read().unwrap(),Value::Unit);
                assert_eq!(vm.events().iter().filter(|e|matches!(e,Event::EnterFunction(_))).count(),3);
            }
        }
    });
}

#[test]
fn distinct_closure_candidates_keep_code_and_capture_evidence_across_calls() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let left=true;let right=true;let callback=if flag {|value:bool| first(value and left)} else {|value:bool| second(value and right)};let result=callback(mark());assumes result==flag;let again=callback(true);assumes again==flag;",
            "let callback=make(flag);let result=apply(callback,mark());assumes result==flag;",
            "let callback=forward(make(flag));let result=apply(callback,mark());assumes result==flag;",
            "let left=first;let right=second;let callback=if flag {|value:bool| left(value)} else {|value:bool| right(value)};let result=callback(mark());assumes result==flag;",
            "let result=(if flag {|| first(mark())} else {|| second(mark())})();assumes result==flag;",
            "var action=first;let callback=if flag {|f:fn(bool)->bool,value:bool| f(value)} else {|f:fn(bool)->bool,value:bool| f(value)};let result=callback(action,do {action=second;mark()});assumes result;",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} typealias Callback=%{{value:bool}} fn(bool)->bool; fn first(value:bool)->bool {{value}} fn second(value:bool)->bool {{not value}} fn mark()->bool {{true}} fn make(flag:bool)->Callback {{let left=true;let right=true;if flag {{|value:bool| first(value and left)}} else {{|value:bool| second(value and right)}}}} fn apply(callback:Callback,value:bool)->bool {{callback(value)}} fn forward(value:T)->T where T {{value}} fn work(flag:bool) {{{body}}} fn entry() {{work({flag});}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit,"{body}");
            let events=executed_names(&package,&executable,vm.events());
            assert_eq!(events.iter().filter(|name|**name=="mark").count(),1,"{body}: {events:?}");
            let selected=if body.starts_with("var action") || flag {"first"} else {"second"};
            assert!(!events.iter().any(|name|name==if selected=="first" {"second"} else {"first"}),"{body}: {events:?}");
            assert_eq!(events.iter().filter(|name|**name==selected).count(),if body.contains("again") {2}else{1});
        }
    }
}

#[test]
fn distinct_closure_failure_candidates_cleanup_only_the_actual_remaining_capture() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    for choice in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct First {{}} struct Second {{}} struct Caller {{}} impl Retire for First {{fn drop(*self) {{}}}} impl Retire for Second {{fn drop(*self) {{}}}} impl Retire for Caller {{fn drop(*self) {{}}}} fn take_first(value:First) {{}} fn take_second(value:Second) {{}} fn reject() {{assumes false;}} fn work(choice:bool) {{let caller=Caller {{}};let first=First {{}};let second=Second {{}};let callback=if choice {{|flag:bool| {{if flag {{take_first(first);}} else {{take_second(second);}}reject();}}}} else {{|flag:bool| {{if not flag {{take_first(first);}} else {{take_second(second);}}reject();}}}};callback(true);}} fn entry() {{work({choice});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
        let expected=if choice {vec!["entry","work","take_first","drop First","reject","drop Second","drop Caller"]}else{vec!["entry","work","take_second","drop Second","reject","drop First","drop Caller"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn distinct_closure_candidates_keep_their_own_captured_enum_domains() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} enum Choice {{left(fn(fn(bool)->bool,bool)->bool),right(fn(fn(bool)->bool,bool)->bool)}} fn first(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn second(action:fn(bool)->bool,value:bool)->bool {{not action(value)}} fn identity(value:bool)->bool {{value}} fn dispatch(value:Choice)->bool {{value match {{.left(action)=>action(identity,true),.right(action)=>action(identity,true)}}}} fn work(flag:bool)->bool {{let left=Choice.left(first);let right=Choice.right(second);let callback=if flag {{|| dispatch(left)}} else {{|| dispatch(right)}};callback()}} fn entry() {{let result=work({flag});assumes result=={flag};}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),vec!["entry","work","dispatch",if flag {"first"} else {"second"},"identity"]);
    }
}

#[test]
fn distinct_closure_candidates_cannot_consume_a_capture_twice() {
    for calls in ["callback();","callback();callback();"] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}} fn consume(value:Ticket) {{}} fn work(flag:bool) {{let left=Ticket {{}};let right=Ticket {{}};let callback=if flag {{|| consume(left)}} else {{|| consume(right)}};{calls}}} fn entry() {{work(true);}}"));
        let result=source_executable(&package,"entry");
        if calls=="callback();" {result.unwrap();}
        else {assert!(result.is_err(),"a candidate set must not reconstruct consumed captures");}
    }
}

#[test]
fn mixed_named_and_closure_candidates_share_one_argument_preparation() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let callback=if flag {first} else {|value:bool| second(value)};let result=callback(mark());assumes result==flag;",
            "let callback=if flag {|value:bool| first(value)} else {second};let result=callback(mark());assumes result==flag;",
            "let callback=make(flag);let result=apply(callback,mark());assumes result==flag;",
            "let callback=build(flag);let result=callback(mark());assumes result==flag;",
            "let callback=if flag {invoke} else {|action:fn(bool)->bool,value:bool| action(value)};var action=first;let result=callback(action,do {action=second;mark()});assumes result;",
            "let callback=if flag {if flag {first} else {second}} else {|value:bool| second(value)};let result=callback(mark());assumes result==flag;",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn first(value:bool)->bool {{value}} fn second(value:bool)->bool {{not value}} fn mark()->bool {{true}} fn invoke(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn make(flag:bool)->fn(bool)->bool {{if flag {{first}} else {{|value:bool| second(value)}}}} fn build(flag:bool)->%{{action:fn(bool)->bool}} fn(bool)->bool {{let action=make(flag);|value:bool|action(value)}} fn apply(action:fn(bool)->bool,value:bool)->bool {{action(value)}} fn work(flag:bool) {{{body}}} fn entry() {{work({flag});}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="mark").count(),1,"{body}: {names:?}");
            let expected=if flag || body.contains("var action") {"first"} else {"second"};
            assert_eq!(names.iter().filter(|name|name.as_str()==expected).count(),1,"{body}: {names:?}");
            assert!(!names.iter().any(|name|name==if expected=="first" {"second"} else {"first"}),"{body}: {names:?}");
        }
    }
}

#[test]
fn mixed_named_and_closure_failure_candidates_cleanup_once() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Argument {{}} struct Caller {{}} impl Retire for Argument {{fn drop(*self) {{}}}} impl Retire for Caller {{fn drop(*self) {{}}}} fn reject(value:Argument) {{assumes false;}} fn work(flag:bool) {{let caller=Caller {{}};let callback=if flag {{reject}} else {{|value:Argument| reject(value)}};callback(Argument {{}});}} fn entry() {{work({flag});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
        assert_eq!(executed_names(&package,&executable,vm.events()),vec!["entry","work","reject","drop Argument","drop Caller"]);
    }
}

#[test]
fn empty_closure_memory_calls_preserve_writes_and_pointer_results() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let callback=|pointer:*bool| {pointer.* = true;pointer};let result=callback(value.ref);result.*",
            "let result=(|pointer:*bool| {pointer.* = true;pointer})(value.ref);result.*",
            "let callback=if flag {set} else {|pointer:*bool| {pointer.* = true;pointer}};let result=callback(value.ref);result.*",
            "let callback=if flag {|pointer:*bool| set(pointer)} else {|pointer:*bool| {pointer.* = true;return pointer;}};let result=callback(value.ref);result.*",
            "let callback=|pointer:*bool| {pointer.* = true;};callback(value.ref);value",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn work(flag:bool)->bool {{let value=false;{body}}} fn entry()->bool {{work({flag})}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{body}");
        }
    }
}

#[test]
fn closure_memory_checks_actual_pointees_and_all_return_candidates() {
    for body in [
        "let pointer=item.ref;let callback=|value:*Item| value.flag.*;let moved=item;callback(pointer)",
        "let callback=if flag {read} else {|value:*Item| value.flag.*};let pointer=item.ref;let moved=item;callback(pointer)",
        "let callback=if flag {|a:*Item,b:*Item| a} else {|a:*Item,b:*Item| b};let pointer=callback(item.ref,other.ref);let moved=item;pointer.flag.*",
        "let callback=if flag {alias} else {|value:*Item| value};let pointer=callback(item.ref);let moved=item;pointer.flag.*",
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Item {{flag:bool}} impl Retire for Item {{fn drop(*self) {{}}}} fn read(value:*Item)->bool {{value.flag.*}} fn alias(value:*Item)->*Item {{value}} fn entry(flag:bool)->bool {{let item=Item {{flag:true}};let other=Item {{flag:false}};{body}}}"));
        let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
        assert!(error.contains("unavailable"),"{body}: {error}");
    }
}

#[test]
fn closure_memory_failure_does_not_own_or_drop_borrowed_pointees() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Item {{flag:bool}} impl Retire for Item {{fn drop(*self) {{}}}} fn reject(value:*Item) {{let seen=value.flag.*;assumes false;}} fn work(flag:bool) {{let item=Item {{flag:true}};let callback=if flag {{reject}} else {{|value:*Item| {{let seen=value.flag.*;assumes false;}}}};callback(item.ref);}} fn entry() {{work({flag});}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
        assert_eq!(executed_names(&package,&executable,vm.events()),if flag {vec!["entry","work","reject","drop Item"]} else {vec!["entry","work","drop Item"]});
    }
}

#[test]
fn owned_closure_captures_coexist_with_borrowed_pointer_arguments() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let expected=flag;let callback=|pointer:*bool| {pointer.* = expected;pointer};let result=callback(value.ref);let again=callback(result);again.*",
            "let callback=build(flag);let result=callback(value.ref);result.*",
            "let left=flag;let right=not flag;let callback=if flag {|pointer:*bool| {pointer.* = left;pointer}} else {|pointer:*bool| {pointer.* = not right;pointer}};let result=callback(value.ref);result.*",
            "let held=Held {};let callback=|pointer:*bool| {consume(held);pointer.* = true;pointer};let result=callback(value.ref);result.* and flag",
            "let held=Held {};let callback=|pointer:*bool| {observe(held);pointer.*};let first=callback(value.ref);let second=callback(value.ref);flag",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}} fn consume(value:Held) {{}} fn observe(value:Held) {{}} fn build(flag:bool)->%{{flag:bool}} fn(*bool)->*bool {{|pointer:*bool| {{pointer.* = flag;pointer}}}} fn work(flag:bool)->bool {{let value=false;{body}}} fn entry()->bool {{work({flag})}}"));
            // The last body intentionally moves an affine capture twice.
            if body.contains("observe") {
                assert!(source_executable(&package,"entry").unwrap_err().contains("unavailable"));
                continue;
            }
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(flag),"{body}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="drop Held").count(),usize::from(body.contains("consume")),"{body}: {names:?}");
        }
    }
}

#[test]
fn owned_closure_memory_failure_cleans_capture_and_caller_in_order() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    for consumed in [true,false] {
        let body=if consumed {"consume(held);let seen=pointer.*;assumes false;"} else {"let seen=pointer.*;assumes false;consume(held);"};
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Held {{}} struct Caller {{}} impl Retire for Held {{fn drop(*self) {{}}}} impl Retire for Caller {{fn drop(*self) {{}}}} fn consume(value:Held) {{}} fn entry() {{let caller=Caller {{}};let held=Held {{}};let value=true;let callback=|pointer:*bool| {{{body}}};callback(value.ref);}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
        assert_eq!(executed_names(&package,&executable,vm.events()),if consumed {vec!["entry","consume","drop Held","drop Caller"]} else {vec!["entry","drop Held","drop Caller"]});
    }
}

#[test]
fn captured_named_callable_memory_preserves_candidate_effects_and_results() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let action=if flag {set} else {clear};let callback=|pointer:*bool| action(pointer);let result=callback(value.ref);result.*",
            "let left=set;let right=clear;let callback=if flag {|pointer:*bool| left(pointer)} else {|pointer:*bool| right(pointer)};let result=callback(value.ref);result.*",
            "let callback=build(flag);let result=callback(value.ref);result.*",
            "let action:fn(*bool)->*bool=forward;let callback=|pointer:*bool|action(set(pointer));let result=callback(value.ref);result.* and flag",
            "var action=set;let callback=|pointer:*bool| {let result=action(pointer);action=clear;result};let first=callback(value.ref);let seen=first.*;let second=callback(value.ref);let after=second.*;assumes not after;seen and flag",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn clear(pointer:*bool)->*bool {{pointer.* = false;pointer}} fn build(flag:bool)->%{{action:fn(*bool)->*bool}} fn(*bool)->*bool {{let action=if flag {{set}} else {{clear}};|pointer:*bool| action(pointer)}} fn forward(value:T)->T where T {{value}} fn work(flag:bool)->bool {{let value=false;{body}}} fn entry()->bool {{work({flag})}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(flag),"{body}");
            let names=executed_names(&package,&executable,vm.events());
            if body.starts_with("var") {
                assert_eq!(names.iter().filter(|name|name.as_str()=="set").count(),1);
                assert_eq!(names.iter().filter(|name|name.as_str()=="clear").count(),1);
            } else if body.contains("forward") {
                assert_eq!(names.iter().filter(|name|name.as_str()=="set").count(),1);
                assert_eq!(names.iter().filter(|name|name.as_str()=="forward").count(),1);
            } else {
                assert!(names.iter().any(|name|name==if flag {"set"} else {"clear"}));
                assert!(!names.iter().any(|name|name==if flag {"clear"} else {"set"}));
            }
        }
    }
}

#[test]
fn captured_memory_callees_cannot_hide_alternative_pointer_results() {
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool} impl Retire for Item {fn drop(*self) {}} fn first(a:*Item,b:*Item)->*Item {a} fn second(a:*Item,b:*Item)->*Item {b} fn entry(flag:bool)->bool {let first_item=Item {flag:true};let second_item=Item {flag:false};let action=if flag {first} else {second};let callback=|a:*Item,b:*Item|action(a,b);let pointer=callback(first_item.ref,second_item.ref);let moved=first_item;pointer.flag.*}");
    assert!(source_executable(&package,"entry").unwrap_err().contains("unavailable"));
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} fn named(value:*bool)->*bool {value} fn entry(flag:bool)->bool {let value=true;let action=if flag {named} else {|value:*bool| {value.* = false;value}};let callback=|value:*bool|action(value);let pointer=callback(value.ref);pointer.*}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [true,false] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|name|name.as_str()=="named").count(),usize::from(flag),"{names:?}");
    }
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool} impl Retire for Item {fn drop(*self) {}} fn first(a:*Item,b:*Item)->*Item {a} fn entry(flag:bool)->bool {let a=Item {flag:true};let b=Item {flag:false};let action=if flag {first} else {|a:*Item,b:*Item|b};let callback=|a:*Item,b:*Item|action(a,b);let pointer=callback(a.ref,b.ref);let moved=b;pointer.flag.*}");
    assert!(source_executable(&package,"entry").unwrap_err().contains("unavailable"));
}

#[test]
fn captured_callable_fields_supply_memory_footprints_and_track_replacements() {
    use comptime_interpretor::{Interpreter,Value};
    for body in [
        "let ops=Ops {action:set};let callback=|pointer:*bool|ops.action(pointer);let pointer=callback(value.ref);pointer.*",
        "let ops=Ops {action:clear};let callback=|pointer:*bool| {while true {ops.action=set;break;}ops.action(pointer)};let pointer=callback(value.ref);pointer.*",
        "let ops=Nested {pair:(set,false)};let callback=|pointer:*bool| {let (action,_)=ops.pair;action(pointer)};let pointer=callback(value.ref);pointer.*",
        "let ops=Ops {action:set};let callback=|pointer:*bool| {let result=ops.action(pointer);ops.action=clear;result};let first=callback(value.ref);let seen=first.*;let second=callback(value.ref);let after=second.*;assumes not after;seen",
        "var ops=Ops {action:clear};let callback=|pointer:*bool| {ops=Ops {action:set};ops.action(pointer)};let pointer=callback(value.ref);pointer.*",
        "let ops=Ops {action:set};let callback=|pointer:*bool| {let saved=ops;let {action}=saved;action(pointer)};let pointer=callback(value.ref);pointer.*",
        "var action=set;let callback=|pointer:*bool| {let values=(action,do {action=clear;false});let (saved,_)=values;saved(pointer)};let pointer=callback(value.ref);pointer.*",
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ops {{action:fn(*bool)->*bool}} struct Nested {{pair:(fn(*bool)->*bool,bool)}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn clear(pointer:*bool)->*bool {{pointer.* = false;pointer}} fn entry()->bool {{let value=false;{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{body}");
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|name|name.as_str()=="set").count(),1,"{body}: {names:?}");
        assert_eq!(names.iter().filter(|name|name.as_str()=="clear").count(),usize::from(body.contains("let second")),"{body}: {names:?}");
    }
}

#[test]
fn captured_memory_fields_invalidate_replaced_code_and_join_changed_targets() {
    for update in ["if flag {ops.action=second;}","if flag {ops=Ops {action:second};}"] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Item {{flag:bool}} impl Retire for Item {{fn drop(*self) {{}}}} struct Ops {{action:fn(*Item,*Item)->*Item}} fn first(a:*Item,b:*Item)->*Item {{a}} fn second(a:*Item,b:*Item)->*Item {{b}} fn entry(flag:bool)->bool {{let a=Item {{flag:true}};let b=Item {{flag:false}};var ops=Ops {{action:first}};let callback=|a:*Item,b:*Item,flag:bool| {{{update}ops.action(a,b)}};let pointer=callback(a.ref,b.ref,flag);let moved=b;pointer.flag.*}}"));
        let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
        assert!(error.contains("unavailable"),"{update}: {error}");
    }
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Ops {action:fn(*bool)->*bool} fn named(value:*bool)->*bool {value} fn entry()->bool {let value=true;var ops=Ops {action:named};let callback=|value:*bool| {ops=Ops {action:|value:*bool| {value.* = false;value}};ops.action(value)};let result=callback(value.ref);result.*}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=comptime_interpretor::Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),comptime_interpretor::Value::Bool(false));
    assert!(!executed_names(&package,&executable,vm.events()).iter().any(|name|name=="named"),"aggregate replacement must call its anonymous code");
}

#[test]
fn aggregate_callable_memory_results_preserve_branch_local_snapshots() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let ops=if flag {Ops {action:set}} else {Ops {action:clear}};ops.action(pointer)",
            "let ops=flag match {true=>Ops {action:set},false=>Ops {action:clear}};let {action}=ops;action(pointer)",
            "let ops=flag match {true if false=>Ops {action:|value:*bool|value},true=>Ops {action:set},false=>Ops {action:clear}};ops.action(pointer)",
            "let (action,_)=if flag {(set,true)} else {(clear,false)};action(pointer)",
            "var action=set;let ops=if flag {Ops {action:action}} else {action=clear;Ops {action:action}};action=clear;ops.action(pointer)",
            "(if flag {Ops {action:set}} else {Ops {action:clear}}).action(pointer)",
            "let ops=if flag {Ops {action:set}} else {return clear(pointer);};ops.action(pointer)",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ops {{action:fn(*bool)->*bool}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn clear(pointer:*bool)->*bool {{pointer.* = false;pointer}} fn work(flag:bool)->bool {{let value=false;let callback=|pointer:*bool,flag:bool| {{{body}}};let result=callback(value.ref,flag);result.*}} fn entry()->bool {{work({flag})}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(flag),"{body}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()==if flag {"set"} else {"clear"}).count(),1,"{body}: {names:?}");
            assert!(!names.iter().any(|name|name==if flag {"clear"} else {"set"}),"{body}: {names:?}");
        }
    }
}

#[test]
fn aggregate_memory_branch_evidence_cannot_hide_unknown_or_dead_targets() {
    for operation in [
        "let ops=if flag {Ops {action:first}} else {Ops {action:second}};ops.action(a,b)",
        "let ops=flag match {true=>Ops {action:first},false=>Ops {action:second}};ops.action(a,b)",
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Item {{flag:bool}} impl Retire for Item {{fn drop(*self) {{}}}} struct Ops {{action:fn(*Item,*Item)->*Item}} fn first(a:*Item,b:*Item)->*Item {{a}} fn second(a:*Item,b:*Item)->*Item {{b}} fn entry(flag:bool)->bool {{let a=Item {{flag:true}};let b=Item {{flag:false}};let callback=|a:*Item,b:*Item,flag:bool| {{{operation}}};let pointer=callback(a.ref,b.ref,flag);let moved=b;pointer.flag.*}}"));
        let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");assert!(error.contains("unavailable"),"{operation}: {error}");
    }
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Ops {action:fn(*bool)->*bool} fn named(value:*bool)->*bool {value} fn entry(flag:bool)->bool {let value=true;let callback=|value:*bool,flag:bool| {let ops=if flag {Ops {action:named}} else {Ops {action:|value:*bool| {value.* = false;value}}};ops.action(value)};let result=callback(value.ref,flag);result.*}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=comptime_interpretor::Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![comptime_interpretor::Value::Bool(flag)]).unwrap(),comptime_interpretor::Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()).iter().filter(|name|name.as_str()=="named").count(),usize::from(flag));
    }
}

#[test]
fn named_aggregate_factories_supply_memory_callable_results_across_recursion() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for factory in ["make(flag)","forward(flag)","recursive(flag)","mutual(flag)","selected(flag)","tupled(flag)"] {
            let body=if factory.starts_with("tupled") {format!("let (action,_)=tupled(flag);action(pointer)")} else {format!("let ops={factory};ops.action(pointer)")};
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ops {{action:fn(*bool)->*bool}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn clear(pointer:*bool)->*bool {{pointer.* = false;pointer}} fn make(flag:bool)->Ops {{if flag {{return Ops {{action:set}};}}Ops {{action:clear}}}} fn forward(flag:bool)->Ops {{make(flag)}} fn recursive(flag:bool)->Ops {{if flag {{return Ops {{action:set}};}}recursive(true)}} fn tupled(flag:bool)->(fn(*bool)->*bool,bool) {{if flag {{(set,true)}} else {{(clear,false)}}}} fn mutual(flag:bool)->Ops {{if flag {{return Ops {{action:set}};}}other(true)}} fn other(flag:bool)->Ops {{if flag {{return Ops {{action:clear}};}}mutual(true)}} fn left_factory()->Ops {{Ops {{action:set}}}} fn right_factory()->Ops {{Ops {{action:clear}}}} fn selected(flag:bool)->Ops {{let factory=if flag {{left_factory}} else {{right_factory}};factory()}} fn work(flag:bool)->bool {{let value=false;let callback=|pointer:*bool,flag:bool| {{{body}}};let result=callback(value.ref,flag);result.*}} fn entry()->bool {{work({flag})}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{factory}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            let expected=flag || factory.starts_with("recursive");
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(expected),"{factory}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()==if expected {"set"} else {"clear"}).count(),1,"{factory}: {names:?}");
            assert!(!names.iter().any(|name|name==if expected {"clear"} else {"set"}),"{factory}: {names:?}");
        }
    }
}

#[test]
fn named_factory_memory_results_keep_all_pointer_targets_and_code_candidates() {
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool} impl Retire for Item {fn drop(*self) {}} struct Ops {action:fn(*Item,*Item)->*Item} fn first(a:*Item,b:*Item)->*Item {a} fn second(a:*Item,b:*Item)->*Item {b} fn make(flag:bool)->Ops {if flag {Ops {action:first}} else {Ops {action:second}}} fn entry(flag:bool)->bool {let a=Item {flag:true};let b=Item {flag:false};let callback=|a:*Item,b:*Item,flag:bool| {let ops=make(flag);ops.action(a,b)};let pointer=callback(a.ref,b.ref,flag);let moved=b;pointer.flag.*}");
    assert!(source_executable(&package,"entry").unwrap_err().contains("unavailable"));
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Ops {action:fn(*bool)->*bool} fn named(value:*bool)->*bool {value} fn make(flag:bool)->Ops {if flag {Ops {action:named}} else {Ops {action:|value:*bool| {value.* = false;value}}}} fn entry(flag:bool)->bool {let value=true;let callback=|value:*bool,flag:bool| {let ops=make(flag);ops.action(value)};let result=callback(value.ref,flag);result.*}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [true,false] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()).iter().filter(|name|name.as_str()=="named").count(),usize::from(flag));
    }
}

#[test]
fn forwarded_callable_memory_results_substitute_evaluated_argument_fields() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "let ops=pack(if flag {set} else {clear});ops.action(pointer)",
            "let ops=identity(pack(if flag {set} else {clear}));ops.action(pointer)",
            "let ops=repack(pack(if flag {set} else {clear}));ops.action(pointer)",
            "let ops=relay(pack(if flag {set} else {clear}),false);ops.action(pointer)",
            "let ops=pick(Ops {action:set},Ops {action:clear},flag);ops.action(pointer)",
            "let action=identity(if flag {set} else {clear});action(pointer)",
            "let ops=with_default(if flag {set} else {clear});ops.action(pointer)",
            "var action=if flag {set} else {clear};let ops=pack_after(action,do {action=clear;false});ops.action(pointer)",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} typealias Action=fn(*bool)->*bool; impl Duplicate for Action {{}} struct Ops {{action:fn(*bool)->*bool}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn clear(pointer:*bool)->*bool {{pointer.* = false;pointer}} fn pack(action:fn(*bool)->*bool)->Ops {{Ops {{action:action}}}} fn repack(ops:Ops)->Ops {{pack(ops.action)}} fn relay(ops:Ops,stop:bool)->Ops {{if stop {{return ops;}}relay(ops,true)}} fn pack_after(action:fn(*bool)->*bool,flag:bool)->Ops {{Ops {{action:action}}}} fn identity(value:T)->T where T {{value}} fn pick(first:T,second:T,flag:bool)->T where T {{if flag {{first}} else {{second}}}} fn with_default(action:fn(*bool)->*bool,.saved:fn(*bool)->*bool=action)->Ops {{Ops {{action:saved}}}} fn work(flag:bool)->bool {{let value=false;let callback=|pointer:*bool,flag:bool| {{{body}}};let result=callback(value.ref,flag);result.*}} fn entry()->bool {{work({flag})}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(flag),"{body}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()==if flag {"set"} else {"clear"}).count(),1,"{body}: {names:?}");
            assert!(!names.iter().any(|name|name==if flag {"clear"} else {"set"}),"{body}: {names:?}");
        }
    }
}

#[test]
fn forwarded_memory_code_keeps_all_inputs_and_returned_pointer_candidates() {
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool} impl Retire for Item {fn drop(*self) {}} struct Ops {action:fn(*Item,*Item)->*Item} fn first(a:*Item,b:*Item)->*Item {a} fn second(a:*Item,b:*Item)->*Item {b} fn pack(action:fn(*Item,*Item)->*Item)->Ops {Ops {action:action}} fn entry(flag:bool)->bool {let a=Item {flag:true};let b=Item {flag:false};let callback=|a:*Item,b:*Item,flag:bool| {let ops=pack(if flag {first} else {second});ops.action(a,b)};let pointer=callback(a.ref,b.ref,flag);let moved=b;pointer.flag.*}");
    assert!(source_executable(&package,"entry").unwrap_err().contains("unavailable"));
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Ops {action:fn(*bool)->*bool} fn named(value:*bool)->*bool {value} fn choose(action:fn(*bool)->*bool,flag:bool)->Ops {if flag {Ops {action:action}} else {Ops {action:named}}} fn entry(flag:bool)->bool {let value=true;let callback=|value:*bool,flag:bool| {let ops=choose(|value:*bool| {value.* = false;value},flag);ops.action(value)};let result=callback(value.ref,flag);result.*}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [true,false] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(!flag));
        assert_eq!(executed_names(&package,&executable,vm.events()).iter().filter(|name|name.as_str()=="named").count(),usize::from(!flag));
    }
}

#[test]
fn code_value_copy_requires_capability_and_no_custom_destructor() {
    for (implementations, reason) in [
        ("", "default preparation consumed a parameter"),
        ("impl Duplicate for Action {} impl Retire for Action {fn drop(*self) {}}", "Copy representation and resource legality"),
    ] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} typealias Action=fn(bool)->bool; {implementations} fn action(value:bool)->bool {{value}} fn choose(action:Action,.saved:Action=action)->Action {{saved}} fn entry()->bool {{let selected=choose(action);selected(true)}}"));
        let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
        assert!(error.contains(reason),"{error}");
    }
}

#[test]
fn recursive_callable_cleanup_backedges_keep_each_runtime_frame() {
    use comptime_interpretor::{Interpreter,Value};
    for reject in [false,true] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Frame {{}} impl Retire for Frame {{fn drop(*self) {{}}}} fn first()->bool {{false}} fn second()->bool {{true}} fn descend(action:fn()->bool,remaining:.u32'builtin)->bool {{let frame=Frame {{}};if remaining==0 {{assumes not {reject};return action();}}descend(second,remaining - 1)}} fn entry()->bool {{descend(first,4)}}"));
        let executable=source_executable(&package,"entry").unwrap();
        assert!(!executable.schedule.aliases.is_empty());
        println!("RECURSIVE_CLEANUP_DUMP={}",executable.schedule);
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if reject {assert_eq!(result.unwrap_err().kind,comptime_interpretor::ErrorKind::ProgramContractRejected);} else {assert_eq!(result.unwrap(),Value::Bool(true));}
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|name|name.as_str()=="descend").count(),5,"{names:?}");
        assert_eq!(names.iter().filter(|name|name.as_str()=="drop Frame").count(),5,"{names:?}");
        assert!(!names.iter().any(|name|name=="first"),"{names:?}");
        assert_eq!(names.iter().filter(|name|name.as_str()=="second").count(),usize::from(!reject),"{names:?}");
    }
}

#[test]
fn recursive_cleanup_aliases_reject_malformed_targets_before_execution() {
    use comptime_interpretor::{ErrorKind,Interpreter};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} fn action()->bool {true} fn descend(action:fn()->bool,remaining:.u32'builtin)->bool {if remaining==0 {return action();}descend(action,remaining - 1)} fn entry()->bool {descend(action,3)}");
    let executable=source_executable(&package,"entry").unwrap();
    assert!(!executable.schedule.aliases.is_empty());
    for mutation in 0..5 {
        let mut schedule=executable.schedule.clone();
        match mutation {
            0=>schedule.aliases[0].target=schedule.aliases[0].context.clone(),
            1=>schedule.aliases[0].target.clear(),
            2=>schedule.aliases.push(schedule.aliases[0].clone()),
            3=>{let alias=&schedule.aliases[0];let target=alias.target.clone();let instance=alias.instance.clone();schedule.functions.retain(|scope|scope.instance!=instance || scope.context!=target);},
            4=>{if let Some(iir::cleanup::ContextKey::Function {callee,..})=schedule.aliases[0].context.last_mut() {*callee=executable.entry.clone();}},
            _=>unreachable!(),
        }
        let error=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&schedule).err().unwrap();
        assert_eq!(error.kind,ErrorKind::InvalidCallPlan,"mutation {mutation}");
    }
}

#[test]
fn higher_order_named_memory_calls_use_prepared_code_contexts() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for body in [
            "apply(if flag {set} else {clear},pointer)",
            "relay(if flag {set} else {clear},pointer)",
            "factory_apply(pointer,flag)",
            "let invoke=if flag {apply} else {relay};invoke(if flag {set} else {clear},pointer)",
            "aggregate(Ops {action:if flag {set} else {clear}},pointer)",
            "recursive(if flag {set} else {clear},pointer,false)",
            "var action=if flag {set} else {clear};after(action,do {action=clear;pointer})",
        ] {
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ops {{action:fn(*bool)->*bool}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn clear(pointer:*bool)->*bool {{pointer.* = false;pointer}} fn apply(action:fn(*bool)->*bool,pointer:*bool)->*bool {{action(pointer)}} fn relay(action:fn(*bool)->*bool,pointer:*bool)->*bool {{apply(action,pointer)}} fn factory(flag:bool)->fn(*bool)->*bool {{if flag {{set}} else {{clear}}}} fn factory_apply(pointer:*bool,flag:bool)->*bool {{apply(factory(flag),pointer)}} fn aggregate(ops:Ops,pointer:*bool)->*bool {{ops.action(pointer)}} fn recursive(action:fn(*bool)->*bool,pointer:*bool,stop:bool)->*bool {{if stop {{return action(pointer);}}recursive(action,pointer,true)}} fn after(action:fn(*bool)->*bool,pointer:*bool)->*bool {{apply(action,pointer)}} fn work(flag:bool)->bool {{let value=false;let callback=|pointer:*bool,flag:bool| {{{body}}};let result=callback(value.ref,flag);result.*}} fn entry()->bool {{work({flag})}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{body}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(flag),"{body}");
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()==if flag {"set"} else {"clear"}).count(),1,"{body}: {names:?}");
            assert!(!names.iter().any(|name|name==if flag {"clear"} else {"set"}),"{body}: {names:?}");
        }
    }
}

#[test]
fn higher_order_closure_memory_inputs_share_single_evaluation() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        for choose in [
            "|action:fn(*bool)->*bool,pointer:*bool|action(pointer)",
            "if flag {apply} else {|action:fn(*bool)->*bool,pointer:*bool|action(pointer)}",
            "|ops:Ops,pointer:*bool|ops.action(pointer)",
        ] {
            let argument=if choose.contains("|ops:") {"Ops {action:choose(flag)}"} else {"choose(flag)"};
            let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} struct Ops {{action:fn(*bool)->*bool}} fn set(pointer:*bool)->*bool {{pointer.* = true;pointer}} fn clear(pointer:*bool)->*bool {{pointer.* = false;pointer}} fn choose(flag:bool)->fn(*bool)->*bool {{if flag {{set}} else {{clear}}}} fn apply(action:fn(*bool)->*bool,pointer:*bool)->*bool {{action(pointer)}} fn work(flag:bool)->bool {{let value=false;let invoke={choose};let result=invoke({argument},value.ref);result.*}} fn entry()->bool {{work({flag})}}"));
            let executable=source_executable(&package,"entry").unwrap_or_else(|error|panic!("{choose}: {error}"));
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(flag));
            let names=executed_names(&package,&executable,vm.events());
            assert_eq!(names.iter().filter(|name|name.as_str()=="choose").count(),1,"{names:?}");
        }
    }
}

#[test]
fn higher_order_memory_keeps_all_returned_pointer_dependencies() {
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {value:bool} impl Retire for Item {fn drop(*self) {}} fn first(a:*Item,b:*Item)->*Item {a} fn second(a:*Item,b:*Item)->*Item {b} fn apply(action:fn(*Item,*Item)->*Item,a:*Item,b:*Item)->*Item {action(a,b)} fn entry(flag:bool)->bool {let a=Item {value:true};let b=Item {value:false};let result=apply(if flag {first} else {second},a.ref,b.ref);let moved=b;result.value.*}");
    let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
    assert!(error.contains("unavailable"),"{error}");
}

#[test]
fn higher_order_memory_contract_rejection_unwinds_once() {
    use comptime_interpretor::{ErrorKind,Interpreter};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Caller {} struct Frame {} impl Retire for Caller {fn drop(*self) {}} impl Retire for Frame {fn drop(*self) {}} fn reject(pointer:*bool)->*bool {let seen=pointer.*;assumes false;pointer} fn apply(action:fn(*bool)->*bool,pointer:*bool)->*bool {let frame=Frame {};action(pointer)} fn entry()->bool {let caller=Caller {};let value=true;let result=apply(reject,value.ref);result.*}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","reject","drop Frame","drop Caller"]);
}

#[test]
fn higher_order_generic_callback_instances_preserve_memory_identity() {
    use comptime_interpretor::{Interpreter,Value};
    for flag in [true,false] {
        let package=compile(&format!("trait Retire {{fn drop(*self);}} trait Duplicate {{}} fn identity(value:T)->T where T {{value}} fn apply(action:fn(*bool)->*bool,pointer:*bool)->*bool {{action(pointer)}} fn entry()->bool {{let value={flag};let result=apply(identity<*bool>,value.ref);result.*}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","identity"]);
    }
}

#[test]
fn higher_order_anonymous_alternative_never_becomes_named_only() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} fn named(pointer:*bool)->*bool {pointer} fn apply(action:fn(*bool)->*bool,pointer:*bool)->*bool {action(pointer)} fn entry(flag:bool)->bool {let value=true;let result=apply(if flag {named} else {|pointer:*bool| {pointer.* = false;pointer}},value.ref);result.*}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [true,false] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|name|name.as_str()=="named").count(),usize::from(flag),"{names:?}");
    }
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {} struct Item {value:bool} impl Retire for Item {fn drop(*self) {}}
        fn first(a:*Item,b:*Item)->*Item {a}
        fn apply(action:fn(*Item,*Item)->*Item,a:*Item,b:*Item)->*Item {action(a,b)}
        fn entry(flag:bool)->bool {let a=Item {value:true};let b=Item {value:false};let result=apply(if flag {first} else {|a:*Item,b:*Item|b},a.ref,b.ref);let moved=b;result.value.*}");
    let error=source_executable(&package,"entry").err().expect("moved enum must be rejected");
    assert!(error.contains("unavailable"),"{error}");
}

#[test]
fn anonymous_record_patterns_transfer_fields_and_clean_omitted_owners_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        fn build(...properties:T)->T where T {properties}
        fn take(item:Item) {}
        fn entry() {let {taken}=build {taken:Item {},omitted:Item {}};take(taken);}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    let names=executed_names(&package,&executable,vm.events());
    assert_eq!(names.iter().filter(|name|name.as_str()=="drop Item").count(),2,"{names:?}");
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        fn build(...properties:T)->T where T {properties}
        fn take(item:Item) {}
        fn entry() {let {taken}=build {taken:Item {}};take(taken);take(taken);}");
    assert!(source_executable(&package,"entry").is_err());
}

#[test]
fn optional_propagation_cleans_locals_and_prepared_arguments_once() {
    use comptime_interpretor::Interpreter;
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        enum Maybe where T {Absent,Present(T)}
        fn missing()->?bool {null}
        fn available()->?Item {Item {}}
        fn later()->bool {true}
        fn consume(item:Item,flag:bool,other:bool)->bool {flag}";
    for (body, trace) in [
        ("let held=Item {};missing()?", vec!["entry","missing","drop Item"]),
        ("consume(Item {},missing()?,later())", vec!["entry","missing","drop Item"]),
        ("let item=available()?;true", vec!["entry","available","drop Item"]),
    ] {
        let package=compile_with_optional(&format!("{prefix} fn entry()->?bool {{{body}}}"),0,Some("input::Maybe"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        vm.invoke_instance(&executable.entry,vec![]).unwrap();
        assert_eq!(executed_names(&package,&executable,vm.events()),trace);
    }
}

#[test]
fn inferred_optional_propagation_completes_before_callback_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        enum Maybe where T {Absent,Present(T)}
        fn lookup(flag:bool)->?bool {if flag {true} else {null}}
        fn apply(action:fn(bool)->?T,flag:bool)->?T where T {action(flag)}";
    for flag in ["true","false"] {
        let source=format!("{prefix} fn entry()->bool {{let result=apply(|flag| {{let item=Item {{}};let value=lookup(flag)?;value}},{flag});result match {{value?=>value,null=>true}}}}");
        let package=compile_with_optional(&source,0,Some("input::Maybe"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","lookup","drop Item"]);
    }
}

#[test]
fn inferred_optional_explicit_missing_return_cleans_local_once() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        enum Maybe where T {Absent,Present(T)}
        fn apply(action:fn(bool)->?T,flag:bool)->?T where T {action(flag)}";
    for flag in ["true","false"] {
        let source=format!("{prefix} fn entry()->bool {{
            let result=apply(|flag| {{let item=Item {{}};if not flag {{return null;}} 9}},{flag});
            result match {{value?=>value==9,null=>true}}}}");
        let package=compile_with_optional(&source,0,Some("input::Maybe"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","apply","drop Item"]);
    }
}

#[test]
fn partial_optional_branches_cleanup_selected_branch_once() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        enum Maybe where T {Absent,Present(T)}
        fn keep(value:?T)->?T where T {value}";
    for body in [
        "if flag {let item=Item {};null} else {let item=Item {};9}",
        "flag match {true=>{let item=Item {};null},false=>{let item=Item {};9}}",
        "when {flag=>{let item=Item {};null},else=>{let item=Item {};9}}",
    ] {
        for flag in ["true","false"] {
            let source=format!("{prefix} fn entry()->bool {{let flag={flag};let result=keep({body});
                result match {{value?=>not flag and value==9,null=>flag}}}}");
            let package=compile_with_optional(&source,0,Some("input::Maybe"));
            let executable=source_executable(&package,"entry").unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
            assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Item","keep"]);
        }
    }
}

#[test]
fn contextual_optional_propagation_cleans_loop_and_argument_owners() {
    use comptime_interpretor::Interpreter;
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        enum Maybe where T {Absent,Present(T)}
        fn empty()->?T where T {null}
        fn later()->bool {true}
        fn consume(item:Item,flag:bool,other:bool)->bool {flag}";
    for body in [
        "consume(Item {},empty()?,later())",
        "while true {let item=Item {};let flag:bool=empty()?;} true",
    ] {
        let package=compile_with_optional(&format!("{prefix} fn entry()->?bool {{{body}}}"),0,Some("input::Maybe"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        vm.invoke_instance(&executable.entry,vec![]).unwrap();
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","empty","drop Item"]);
    }
}

#[test]
fn inferred_optional_return_tail_retry_preserves_single_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        enum Maybe where T {Absent,Present(T)}
        fn touch()->bool {true}
        fn lookup(flag:bool)->?bool {if flag {true} else {null}}
        fn apply(action:fn(bool)->?T,flag:bool)->?T where T {action(flag)}";
    for propagation in ["", "lookup(flag)?;"] {
        for flag in ["true","false"] {
            let source=format!("{prefix} fn entry()->bool {{let result=apply(|flag| {{
                let item=Item {{}};touch();{propagation}if flag {{return 9;}} null
            }},{flag});result match {{value?=>{flag} and value==9,null=>not {flag}}}}}");
            let package=compile_with_optional(&source,0,Some("input::Maybe"));
            let executable=source_executable(&package,"entry").unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
            let expected=if propagation.is_empty() {vec!["entry","apply","touch","drop Item"]}
                else {vec!["entry","apply","touch","lookup","drop Item"]};
            assert_eq!(executed_names(&package,&executable,vm.events()),expected);
        }
    }
}

#[test]
fn concrete_trait_memory_calls_preserve_borrowed_sources_and_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {}
        trait Replicate {fn clone(*self)->Self;}
        struct Item {flag:bool}
        impl Retire for Item {fn drop(*self) {}}
        impl Replicate for Item {fn clone(*self)->Self {Item {flag:self.flag}}}
        fn replicate(value:*T)->T where T :- Replicate {value.*.as(Replicate).clone()}
        fn entry()->bool {let source=Item {flag:true};let first=source.as(Replicate).clone();
            let second=replicate(source.ref);source.flag and first.flag and second.flag}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","clone","replicate","clone","drop Item","drop Item","drop Item"]);
}

#[test]
fn concrete_trait_memory_calls_check_writes_and_returned_aliases() {
    use comptime_interpretor::Value;
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {}
        trait Access {fn set(*self,value:bool);fn view(*self)->*bool;}
        struct Item {flag:bool}
        impl Retire for Item {fn drop(*self) {}}
        impl Access for Item {
            fn set(*self,value:bool) {self.*.flag = value;}
            fn view(*self)->*bool {self.*.flag.ref}
        }
        fn consume(value:Item) {}";
    let package=compile(&format!("{prefix} fn entry()->bool {{let source=Item {{flag:false}};
        source.as(Access).set(true);let pointer=source.as(Access).view();pointer.* and source.flag}}"));
    assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap(),Value::Bool(true));
    let package=compile(&format!("{prefix} fn entry()->bool {{let source=Item {{flag:false}};
        let pointer=source.as(Access).view();consume(source);pointer.*}}"));
    let failure=source_executable(&package,"entry").unwrap_err();
    assert!(failure.contains("unavailable") || failure.contains("moved"),"{failure}");
}

#[test]
fn concrete_trait_memory_calls_do_not_erase_interface_predicates() {
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {}
        trait Read {fn read(*self,flag:bool)->bool where requires flag;}
        struct Item {flag:bool}
        impl Read for Item {fn read(*self,flag:bool)->bool {self.flag}}
        fn entry()->bool {let item=Item {flag:true};item.as(Read).read(true)}");
    let failure=source_executable(&package,"entry").unwrap_err();
    assert!(failure.contains("composed memory contract"),"{failure}");
}

#[test]
fn concrete_trait_memory_calls_specialize_default_and_generic_methods() {
    use comptime_interpretor::Value;
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {}
        trait Access where T {
            fn read(*self)->T;
            fn fallback(*self)->T {self.read()}
            fn echo(*self,value:U)->U where U {value}
        }
        struct Item {flag:bool}
        impl Retire for Item {fn drop(*self) {}}
        impl Access<bool> for Item {fn read(*self)->bool {self.flag}}
        fn entry()->bool {let item=Item {flag:true};
            item.as(Access<bool>).fallback() and item.as(Access<bool>).echo(true)}");
    assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100_000).unwrap(),Value::Bool(true));
}

#[test]
fn captured_drop_indirect_access_requires_pointer_provenance() {
    use elaboration::verification::{PlaceId, ownership::{Place, Shape, State}};
    let package = compile("trait Retire {fn drop(*self);} struct Link {target:*bool} impl Retire for Link {fn drop(*self) {let value:bool=self.target;}}");
    world(&package, |world, items| {
        let subject = nominal(&package, "Link", vec![]);
        let Resolution::Selected(selected) = luna::protocols::select_drop(
            &package, world, items, &subject, Budget::default()).unwrap() else {panic!("selected")};
        let root = PlaceId {owner:100, local:1};
        let receiver = Place {root, fields:vec![]};
        let mut state = State::default();
        state.declare(root, &Shape::Fields(vec![Shape::Value]), true, &mut 1000).unwrap();
        let before = state.clone();
        let error = luna::protocols::check_drop_access(
            &package, world, &selected, &state, &receiver, &mut 10000).unwrap_err();
        assert!(error.contains("indirect Drop access"), "{error}");
        assert_eq!(state, before);
    });
}

#[test]
fn indirect_drop_access_checks_every_pointer_alternative_and_snapshot() {
    use std::collections::{BTreeMap, BTreeSet};
    use elaboration::verification::{PlaceId, ownership::{Place, Shape, State}, drop::{PointerMemory, check_access_with_memory}};
    let package=compile("trait Retire {fn drop(*self);} struct Link {target:**bool} impl Retire for Link {fn drop(*self) {let observed:bool=self.target;}}");
    world(&package, |world, items| {
        let subject=nominal(&package,"Link",vec![]);
        let Resolution::Selected(selected)=luna::protocols::select_drop(&package,world,items,&subject,Budget::default()).unwrap() else {panic!("selected")};
        let root=|local| PlaceId {owner:100,local};
        let place=|local| Place {root:root(local),fields:vec![]};
        let receiver=place(0);
        let mut state=State::default();
        state.declare(root(0),&Shape::Fields(vec![Shape::Value]),true,&mut 1000).unwrap();
        for index in 1..4 {state.declare(root(index),&Shape::Value,true,&mut 1000).unwrap();}
        let boolean=nir::Term::Primitive(nir::Primitive::Bool);
        let mut types=BTreeMap::from([(root(0),subject),(root(1),nir::Term::Pointer(Box::new(boolean.clone()))),(root(2),boolean.clone()),(root(3),boolean)]);
        let mut slots=BTreeMap::from([
            (Place {root:root(0),fields:vec![0]},BTreeSet::from([place(1)])),
            (place(1),BTreeSet::from([place(2),place(3)])),
        ]);
        let check=|state:&State, types:&BTreeMap<PlaceId,nir::Term>, slots:&BTreeMap<Place,BTreeSet<Place>>, retired:&[Place], fuel:&mut usize|
            check_access_with_memory(&package,world,&selected,state,&receiver,Some(PointerMemory {types,slots,retired}),fuel);
        let before=state.clone();
        let checked=check(&state,&types,&slots,&[],&mut 10000).unwrap();
        assert!(checked.targets.contains(&BTreeSet::from([place(2),place(3)])));
        assert!(checked.to_string().contains("(targets (access"));
        assert_eq!(state,before);
        assert!(check(&state,&types,&slots,&[place(3)],&mut 10000).unwrap_err().contains("earlier cleanup"));
        assert!(check(&state,&types,&slots,&[],&mut 0).unwrap_err().contains("budget"));
        state.take(&place(3)).unwrap();
        assert!(check(&state,&types,&slots,&[],&mut 10000).unwrap_err().contains("unavailable"));
        state.install(&place(3)).unwrap();
        types.insert(root(3),nir::Term::Primitive(nir::Primitive::U8));
        assert!(check(&state,&types,&slots,&[],&mut 10000).unwrap_err().contains("type disagrees"));
        types.insert(root(3),nir::Term::Primitive(nir::Primitive::Bool));
        slots.remove(&place(1));
        assert!(check(&state,&types,&slots,&[],&mut 10000).unwrap_err().contains("provenance"));
        slots.insert(place(1),BTreeSet::new());
        assert!(check(&state,&types,&slots,&[],&mut 10000).unwrap_err().contains("provenance"));
    });
}

#[test]
fn indirect_drop_local_cleanup_and_replacement_use_live_pointer_contents() {
    use comptime_interpretor::Value;
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}}";
    for body in [
        "let flag=false;do {let watcher=Watch {target:flag.ref};} flag",
        "let first=false;let second=false;do {var watcher=Watch {target:first.ref};watcher=Watch {target:second.ref};} first and second",
        "let first=false;let second=false;do {var watcher=Watch {target:first.ref};watcher.target=second.ref;} not first and second",
    ] {
        let package=compile(&format!("{prefix} fn entry()->bool {{{body}}}"));
        assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100000).unwrap(),Value::Bool(true),"{body}");
    }
}

#[test]
fn indirect_drop_rejects_dead_targets_and_reversed_cleanup_dependencies() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Item {flag:bool} fn consume(value:Item) {} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}}";
    for body in [
        "let item=Item {flag:false};let watcher=Watch {target:item.flag.ref};consume(item);true",
        "let early=false;var watcher=Watch {target:early.ref};let late=false;watcher.target=late.ref;true",
        "let early=false;var watcher=Watch {target:early.ref};let item=Item {flag:false};watcher.target=item.flag.ref;consume(item);watcher=Watch {target:early.ref};true",
    ] {
        let package=compile(&format!("{prefix} fn entry()->bool {{{body}}}"));
        let error=luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100000).unwrap_err();
        assert!(error.contains("unavailable") || error.contains("pointer provenance"),"{body}: {error}");
    }
}

#[test]
fn indirect_drop_field_cleanup_respects_reverse_field_order() {
    use comptime_interpretor::Value;
    for (fields, accepted) in [("value:bool,watch:Watch",true),("watch:Watch,value:bool",false)] {
        let source=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} struct Watch {{target:*bool}} impl Retire for Watch {{fn drop(*self) {{self.target.*.* = true;}}}} struct Pair {{{fields}}} fn entry()->bool {{let external=false;var pair=Pair {{value:false,watch:Watch {{target:external.ref}}}};pair.watch.target=pair.value.ref;true}}");
        let package=compile(&source);
        let result=luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100000);
        if accepted { assert_eq!(result.unwrap(),Value::Bool(true)); }
        else { assert!(result.unwrap_err().contains("earlier cleanup")); }
    }
}

#[test]
fn indirect_drop_temporaries_keep_evaluated_pointer_origins() {
    use comptime_interpretor::Value;
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}} struct Bundle {watch:Watch,value:bool}";
    for body in [
        "let flag=false;Watch {target:flag.ref};flag",
        "let flag=false;let watcher=Watch {target:flag.ref};watcher;flag",
        "let flag=false;let _=Watch {target:flag.ref};flag",
        "let flag=false;let selected=Bundle {watch:Watch {target:flag.ref},value:true}.value;flag and selected",
        "let first=false;let second=false;var pointer=first.ref;let selected=Bundle {watch:Watch {target:pointer},value:do {pointer=second.ref;true}}.value;first and not second and selected",
        "let flag=false;var again=true;while again {again=false;Watch {target:flag.ref};}flag",
    ] {
        let package=compile(&format!("{prefix} fn entry()->bool {{{body}}}"));
        assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100000).unwrap(),Value::Bool(true),"{body}");
    }
}

#[test]
fn indirect_drop_temporaries_reject_later_invalidation() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Item {flag:bool} fn consume(value:Item) {} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}} struct Bundle {watch:Watch,value:bool}";
    for body in [
        "let item=Item {flag:false};Bundle {watch:Watch {target:item.flag.ref},value:do {consume(item);true}}.value;true",
        "let item=Item {flag:false};let watcher=Watch {target:item.flag.ref};consume(item);watcher;true",
    ] {
        let package=compile(&format!("{prefix} fn entry()->bool {{{body}}}"));
        let error=luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100000).unwrap_err();
        assert!(error.contains("unavailable") || error.contains("provenance"),"{body}: {error}");
    }
}

#[test]
fn indirect_drop_temporaries_cleanup_once_on_early_exit() {
    use comptime_interpretor::{Interpreter, Value, ErrorKind};
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}} struct Bundle {watch:Watch,value:bool} fn accept(first:bool,second:bool) {}";
    for (body, rejected) in [
        ("let flag=false;accept(Bundle {watch:Watch {target:flag.ref},value:true}.value,do {return true;true});false",false),
        ("let flag=false;while true {accept(Bundle {watch:Watch {target:flag.ref},value:true}.value,do {break;true});}flag",false),
        ("let flag=false;accept(Bundle {watch:Watch {target:flag.ref},value:true}.value,do {assumes false;true});false",true),
    ] {
        let package=compile(&format!("{prefix} fn entry()->bool {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);}
        else {assert_eq!(result.unwrap(),Value::Bool(true));}
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Watch"],"{body}");
    }
}

#[test]
fn indirect_drop_prepared_components_cleanup_once_without_entering_unfinished_calls() {
    use comptime_interpretor::{Interpreter, Value, ErrorKind};
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}} struct Bundle {watch:Watch,value:bool} impl Retire for Bundle {fn drop(*self) {}} fn accept(first:Watch,second:bool)->Watch {first}";
    for (body, rejected) in [
        ("let flag=false;accept(Watch {target:flag.ref},do {return true;true});false",false),
        ("let flag=false;Bundle {watch:Watch {target:flag.ref},value:do {return true;true}};false",false),
        ("let flag=false;while true {accept(Watch {target:flag.ref},do {break;true});}flag",false),
        ("let flag=false;accept(Watch {target:flag.ref},do {assumes false;true});false",true),
        ("let flag=false;Bundle {watch:Watch {target:flag.ref},value:do {assumes false;true}};false",true),
    ] {
        let package=compile(&format!("{prefix} fn entry()->bool {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|error| panic!("{body}: {error}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if rejected {assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);}
        else {assert_eq!(result.unwrap(),Value::Bool(true));}
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Watch"],"{body}");
    }
}

#[test]
fn indirect_drop_prepared_components_preserve_snapshot_order_and_reject_dead_targets() {
    use comptime_interpretor::{Interpreter, Value};
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct Item {flag:bool} fn consume(value:Item) {} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}} struct Marker {} impl Retire for Marker {fn drop(*self) {}} struct Bundle {watch:Watch,marker:Marker,value:bool} impl Retire for Bundle {fn drop(*self) {}}";
    let package=compile(&format!("{prefix} fn entry()->bool {{let first=false;let second=false;var pointer=first.ref;while true {{Bundle {{watch:Watch {{target:pointer}},marker:Marker {{}},value:do {{pointer=second.ref;break;true}}}};}}first and not second}}"));
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Marker","drop Watch"]);
    let package=compile(&format!("{prefix} fn entry()->bool {{let item=Item {{flag:false}};Bundle {{watch:Watch {{target:item.flag.ref}},marker:Marker {{}},value:do {{consume(item);return true;true}}}};false}}"));
    let error=source_executable(&package,"entry").err().expect("dead target must reject");
    assert!(error.contains("unavailable"),"{error}");
}

#[test]
fn indirect_drop_checked_results_keep_origins_until_postcondition_rejection() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    for result in ["result:Watch","Watch"] {
        for body in ["return Watch {target:flag.ref};","Watch {target:flag.ref}","let local=flag;return Watch {target:local.ref};"] {
            let mut package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} struct Watch {{target:*bool}} impl Retire for Watch {{fn drop(*self) {{self.target.*.* = true;}}}} fn entry(flag:bool)->{result} where ensures true {{{body}}}"));
            // Exercise runtime rejection after source contract checking, as in
            // the existing postcondition cleanup fixtures.
            let function=package.functions.iter_mut().find(|f|f.name=="entry").unwrap();
            function.postconditions[0].kind=nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
            let executable=source_executable(&package,"entry").unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(false)]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
            assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Watch"],"{result}, {body}");
        }
    }
}

#[test]
fn named_checked_pointer_result_supports_observation_before_rejection() {
    use comptime_interpretor::{ErrorKind, Event, Interpreter, Value};
    use nir::program::{Expr, ExprKind as K, Literal};
    let mut package=compile("trait Duplicate {} trait Retire {fn drop(*self);} struct Watch {target:*bool} impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}} fn entry(flag:bool)->result:Watch where ensures true {return Watch {target:flag.ref};}");
    let declaration=package.structs.iter().find(|s|s.name=="Watch").unwrap().declaration;
    let function=package.functions.iter_mut().find(|f|f.name=="entry").unwrap();
    let nir::Term::Function {result,..}=&function.signature else {panic!("function")};
    let result_type=(**result).clone();
    let binding=function.result_binding.unwrap();
    let source=function.postconditions[0].source;
    let mut final_check=function.postconditions[0].clone();
    final_check.source.start+=1;
    final_check.kind=K::Literal(Literal::Bool(false));
    function.postconditions[0].kind=K::ReadPointer(Box::new(Expr {
        source,
        ty:nir::Term::Pointer(Box::new(nir::Term::Primitive(nir::Primitive::Bool))),
        kind:K::Field {value:Box::new(Expr {source,ty:result_type,kind:K::Local(binding)}),declaration,index:0},
    }));
    function.postconditions.push(final_check);
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Watch"]);
        assert_eq!(vm.events().iter().filter(|event|matches!(event,Event::CheckPredicate {..})).count(),if flag {2} else {1});
    }
}

#[test]
fn checked_result_origins_do_not_revive_retired_body_locals() {
    for result in ["result:Watch", "Watch"] {
        let mut package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} struct Watch {{target:*bool}} impl Retire for Watch {{fn drop(*self) {{self.target.*.* = true;}}}} fn entry()->{result} where ensures true {{let local=false;Watch {{target:local.ref}}}}"));
        let function=package.functions.iter_mut().find(|f|f.name=="entry").unwrap();
        function.postconditions[0].kind=nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
        let error=source_executable(&package,"entry").err().expect("body-local pointee must not survive the body");
        assert!(error.contains("unavailable") || error.contains("provenance"),"{error}");
    }
}

#[test]
fn resource_checked_returns_reject_addresses_of_local_storage() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);} struct View {target:*bool} fn choose(flag:bool,left:*bool,right:*bool)->*bool {if flag {left} else {right}}";
    for function in [
        "fn entry()->*bool {let local=true;local.ref}",
        "fn entry()->*bool {let local=true;return local.ref;}",
        "fn entry(value:bool)->*bool {value.ref}",
        "fn entry()->View {let local=true;View {target:local.ref}}",
        "fn entry()->View {let local=true;return View {target:local.ref};}",
        "fn entry()->*bool {true match {value=>value.ref}}",
        "fn entry(flag:bool)->*bool {if flag {let local=true;local.ref} else {let other=false;other.ref}}",
        "fn entry(flag:bool,external:*bool)->*bool {let local=true;if flag {local.ref} else {external}}",
        "fn entry(flag:bool,external:*bool)->*bool {let local=true;var selected=external;if flag {selected=local.ref;}selected}",
        "fn entry(flag:bool,external:*bool)->*bool {let local=true;flag match {true=>local.ref,false=>external}}",
        "fn entry(flag:bool,external:*bool)->View {let local=true;if flag {View {target:local.ref}} else {View {target:external}}}",
        "fn entry(flag:bool,external:*bool)->*bool {let local=true;choose(flag,local.ref,external)}",
        "fn entry()->*bool {let outer=true;var selected=outer.ref;do {let inner=false;selected=inner.ref;};selected}",
        "fn entry()->View {let outer=true;var value=View {target:outer.ref};do {let inner=false;value.target=inner.ref;};value}",
    ] {
        let package=compile(&format!("{prefix} {function}"));
        let error=source_executable(&package,"entry").err().unwrap_or_else(|| panic!("escaped local storage: {function}"));
        assert!(error.contains("outside its local scope"),"{function}: {error}");
    }
}

#[test]
fn resource_checked_returns_keep_borrowed_external_targets() {
    use comptime_interpretor::Value;
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} struct Cell {flag:bool} fn identity(pointer:*bool)->*bool {return pointer;} fn view(cell:*Cell)->*bool {cell.flag} fn entry()->bool {let cell=Cell {flag:true};let pointer=identity(view(cell.ref));pointer.*}");
    assert_eq!(luna::execution::interpret_with_protocols(&package,"entry","Duplicate","Retire",100000).unwrap(),Value::Bool(true));
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} fn entry(pointer:*bool)->*bool {if false {let local=true;return local.ref;}pointer}");
    source_executable(&package,"entry").unwrap();
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} fn entry(pointer:*bool)->*bool {var selected=pointer;do {let inner=true;selected=inner.ref;};selected=pointer;selected}");
    source_executable(&package,"entry").unwrap();
}

#[test]
fn resource_checked_uncertain_or_retired_targets_never_gain_access_permission() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}";
    for function in [
        "fn entry(flag:bool,unknown:*bool)->bool {let local=true;let selected=if flag {local.ref} else {unknown};selected.*}",
        "fn entry(flag:bool,unknown:*bool)->bool {let local=true;var selected=local.ref;if flag {selected=unknown;}selected.*}",
        "fn entry()->bool {let outside=false;var pointer=outside.ref;var turn=false;while true {let local=true;if turn {return pointer.*;}pointer=local.ref;turn=true;}false}",
    ] {
        let package=compile(&format!("{prefix} {function}"));
        let error=source_executable(&package,"entry").err().unwrap_or_else(|| panic!("invented pointer permission: {function}"));
        assert!(error.contains("unavailable") || error.contains("provenance"),"{function}: {error}");
    }
}

#[test]
fn representation_copy_deliveries_are_published_separately_from_moves() {
    use comptime_interpretor::{Interpreter,Value,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Point {flag:bool} impl Duplicate for Point {}
        fn entry()->bool {var original=Point {flag:true};let first=original;let second=original;original.flag=false;first.flag and second.flag and not original.flag}");
    let executable=source_executable(&package,"entry").unwrap();
    let index=executable.schedule.functions.iter().position(|scope|scope.instance==executable.entry && scope.context.is_empty()).unwrap();
    let scope=&executable.schedule.functions[index];
    assert!(!scope.copies.is_empty());
    assert!(scope.to_string().contains("(copies (representation-copy"));
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    for conflict in [false,true] {
        let mut schedule=executable.schedule.clone();
        let scope=&mut schedule.functions[index];
        let operation=scope.copies[0].clone();
        if conflict {scope.transfers.push(operation);} else {scope.copies.push(operation);}
        let error=Interpreter::from_iir(&executable.graph,Default::default()).unwrap().with_cleanup_schedule(&schedule).err().unwrap();
        assert_eq!(error.kind,ErrorKind::InvalidCallPlan);
    }
}

#[test]
fn cmtt_match_driven_generation_preserves_runtime_cleanup_order() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum Step {emit,done}
        fn make()->Held {Held {}}
        pure comptime fn build(first:Step)-><|-.void'builtin> {
            var step=first;
            var code='(do {});
            while true {
                step match {
                    .emit=>{
                        const previous=code;
                        code='(do {~<previous>;let held=make();});
                        step=.done;
                        continue;
                    },
                    .done=>{return code;},
                };
            }
        }
        fn entry(){~<build(.emit)>;~<build(.emit)>;}
        fn empty(){~<build(.done)>;}");
    assert!(!package.enums.iter().any(|item|item.name=="<generator-match-packet>"));
    for (name,expected) in [
        ("entry",vec!["entry","make","drop Held","make","drop Held"]),
        ("empty",vec!["empty"]),
    ] {
        let executable=source_executable(&package,name).unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn cmtt_let_else_selects_resources_without_executing_quotes() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        enum Input {none,some(true'type)}
        fn make()->Held {Held {}}
        pure comptime fn build(input:Input)-><|-.void'builtin> {
            let .some(enabled)=input else {return '(do {});};
            let true=enabled else {return '(do {});};
            '(do {let held=make();})
        }
        fn entry(){~<build(.none)>;~<build(.some(false))>;~<build(.some(true))>;}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","make","drop Held"]);
}

#[path="support/inline_return.rs"]
mod inline_return_boundary;

#[test]
fn inline_return_boundary_cleans_early_and_tail_scopes_once() {
    use comptime_interpretor::{Interpreter,Value};
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        fn run(flag:true'type)->true'type {
            let outer=make();
            while true {
                let inner=make();
                if flag {return true;}
                break;
            }
            false
        }";
    let mut package=compile(source);
    inline_return_boundary::wrap(&mut package,"run");
    let executable=source_executable(&package,"run").unwrap();
    for flag in [true,false] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(flag));
        assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","make","drop Held","drop Held"]);
    }
}

#[test]
fn inline_return_boundary_transfers_owned_results_without_retiring_them() {
    use comptime_interpretor::{Interpreter,Value};
    let mut package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        fn choose(flag:true'type)->Held {
            let first=make();
            let second=make();
            if flag {return first;}
            second
        }
        fn run(flag:true'type){let returned=choose(flag);}");
    inline_return_boundary::wrap(&mut package,"choose");
    let executable=source_executable(&package,"run").unwrap();
    for flag in [true,false] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),["run","choose","make","make","drop Held","drop Held"]);
    }
}

#[test]
fn quote_function_arguments_and_owned_returns_keep_cleanup_obligations() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        quote fn choose(first:Held,second:Held,flag:true'type)->Held {
            if flag {return first;}
            second
        }
        quote fn ignore(unused:Held) {}
        fn run(flag:true'type) {let returned=choose(make(),make(),flag);ignore(make());}");
    assert!(!package.functions.iter().any(|f|f.name=="choose" || f.name=="ignore"));
    let executable=source_executable(&package,"run").unwrap();
    for flag in [true,false] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),
            ["run","make","make","drop Held","make","drop Held","drop Held"]);
    }
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        quote fn duplicate(item:Held)->(Held,Held) {(item,item)}
        fn run(item:Held){duplicate(item);}");
    assert!(source_executable(&package,"run").is_err(),"expansion cannot duplicate affine arguments");
}

#[test]
fn quote_defaults_observe_owned_arguments_without_copying_responsibility() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {flag:true'type} impl Retire for Held {fn drop(*self) {}}
        fn make(flag:true'type)->Held {Held {flag:flag}}
        quote fn choose(item:Held,.flag:true'type=item.flag)->Held {
            if flag {return item;}
            item
        }
        fn run(){let a=choose(make(true));let b=choose(make(false),.flag=true);}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","make","drop Held","drop Held"]);
}

#[test]
fn generic_quote_functions_preserve_affine_transfer_through_generic_callers() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        quote fn identity(value:T)->T where T {return value;}
        pure fn forward(value:T)->T where T {identity(value)}
        fn run(){let first=forward(make());let second=identity(make());}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","forward","make","drop Held","drop Held"]);
}

#[test]
fn inherent_quote_methods_transfer_owned_receivers_and_clean_unused_arguments() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        impl Held {quote fn keep(self,unused:Held)->Self {self}}
        fn make()->Held {Held {}}
        fn run(){let kept=make().keep(make());}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","make","drop Held","drop Held"]);
}

#[test]
fn generic_cmtt_generators_keep_resource_identity_through_composition() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        pure comptime fn forward(code:<x:T |- T>)-><x:T |- T> where T {code}
        pure comptime fn compose(code:<x:T |- T>)-><x:T |- T> where T {forward(code)}
        fn make()->Held {Held {}}
        fn run(){let held=make();const code=compose('[resource=held](held));let transferred=~<code>;}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","drop Held"]);
}

#[test]
fn cmtt_default_code_keeps_affine_identity_without_copying_ownership() {
    use comptime_interpretor::{Interpreter,Value};
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        pure comptime fn forward(first:<x:T |- T>,.second:<x:T |- T> = first)-><x:T |- T> where T {second}
        fn make()->Held {Held {}}
        fn run(){let held=make();const code=forward('[x=held](held));let transferred=~<code>;}
        ";
    let package=compile(source);
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","drop Held"]);
    let duplicate=compile(&source.replace("let transferred=~<code>;","let first=~<code>;let second=~<code>;"));
    assert!(source_executable(&duplicate,"run").is_err());
}

#[test]
fn cmtt_spread_configuration_emits_runtime_resource_scope_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        pure comptime fn packet()->(true'type,true'type) {(true,false)}
        pure comptime fn build(first:true'type,second:true'type)->< |- .void'builtin> {
            if first and not second {'(do {let held=make();})} else {'(do {})}
        }
        fn run(){~<build(...packet())>;}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","drop Held"]);
}

#[test]
fn cmtt_property_configuration_preserves_generated_resource_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        pure comptime fn schema(...fields:T)->T where T {fields}
        const SAMPLE=schema {enabled:true};typealias Options=SAMPLE'type;
        pure comptime fn build(...options:Options)->< |- .void'builtin> {
            if options.enabled {'(do {let held=make();})} else {'(do {})}
        }
        fn run(){~<build {enabled:true}>;~<build {enabled:false}>;}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","drop Held"]);
}

#[test]
fn quote_variadic_packets_and_properties_transfer_each_resource_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        fn packet()->(Held,Held) {(make(),make())}
        quote fn pack(...items:T)->T where T {return items;}
        fn run(){let values=pack(...packet());let properties=pack {left:make(),right:make()};}");
    assert!(!package.functions.iter().any(|f|f.name=="pack"));
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["run","packet","make","make","make","make","drop Held","drop Held","drop Held","drop Held"]);
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        quote fn duplicate(...items:(Held,Held))->(Held,Held) {let ignored=items;items}
        fn run(first:Held,second:Held){duplicate(first,second);}");
    assert!(source_executable(&package,"run").is_err(),"variadic expansion must retain affine moves");
}

#[test]
fn trait_quote_selection_transfers_resources_without_runtime_method_calls() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        trait Keep {quote fn keep(self,unused:Self)->Self;}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        impl Keep for Held {quote fn keep(self,other:Self)->Self {return self;}}
        fn make()->Held {Held {}}
        fn run(){let kept=make().as(Keep).keep(make());}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["run","make","make","drop Held","drop Held"]);
}

#[test]
fn late_trait_quote_expansion_preserves_affine_cleanup_through_generic_callers() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        trait Keep {quote fn keep(self,unused:Self)->Self;}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        impl Keep for Held {quote fn keep(self,other:Self)->Self {return self;}}
        fn keep(value:T,other:T)->T where T :- Keep {value.as(Keep).keep(other)}
        fn make()->Held {Held {}}
        fn run(){let kept=keep(make(),make());}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["run","make","make","keep","drop Held","drop Held"]);
}

#[test]
fn late_quote_cleanup_rebuilds_layouts_when_destructors_add_generic_roots() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        trait Keep {quote fn keep(self)->Self;}
        trait Observe {quote fn observe(*self)->true'type {true}}
        struct Held {} impl Observe for Held {}
        fn observe(value:*T)->true'type where T :- Observe {value.as(Observe).observe()}
        impl Retire for Held {fn drop(*self) {observe(self);}}
        impl Keep for Held {quote fn keep(self)->Self {return self;}}
        fn keep(value:T)->T where T :- Keep {value.as(Keep).keep()}
        quote fn identity(value:Held)->Held {value}
        fn make()->Held {Held {}}
        fn run(){let first=identity(make());let second=keep(make());}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["run","make","make","keep","drop Held","observe","drop Held","observe"]);
}

#[test]
fn local_const_nominal_data_materializes_independent_runtime_resources() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        pure comptime fn configured()->Held {Held {}}
        fn run(){const config=configured();let first=config;let second=config;}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","drop Held","drop Held"]);
}

#[test]
fn local_const_callable_identity_transfers_affine_arguments_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {} impl Retire for Held {fn drop(*self) {}}
        fn make()->Held {Held {}}
        fn consume(item:Held) {}
        fn run(){const action=consume;action(make());}");
    let executable=source_executable(&package,"run").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["run","make","consume","drop Held"]);
}

#[test]
fn quote_parameter_protocols_transfer_owned_lambda_and_implicit_inputs_once() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {} struct Item {}
        impl Retire for Item {fn drop(*self) {}}
        fn make()->Item {Item {}} fn consume(implicit item:Item) {}";
    for (body,expected) in [
        ("quote fn apply(lambda action:%{item:Item} fn()) {action();}
          fn entry() {let item=make();apply() do {consume(.item=item);};}",
         vec!["entry","make","consume","drop Item"]),
        ("quote fn apply(implicit item:Item,lambda action:fn(Item)) {action(item);}
          fn entry() {apply(.item=make()) do {consume(.item=_0);};}",
         vec!["entry","make","consume","drop Item"]),
        ("quote fn apply(implicit item:Item,lambda action:fn(Item)) {return;}
          fn entry() {apply(.item=make()) do {consume(.item=_0);};}",
         vec!["entry","make","drop Item"]),
    ] {
        let package=compile(&format!("{prefix}{body}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
    let package=compile(&format!("{prefix}
        quote fn apply(implicit item:Item,lambda action:fn(Item)) {{action(item);}}
        fn entry() {{apply(.item=make()) do {{consume(.item=_0);consume(.item=_0);}};}}"));
    assert!(source_executable(&package,"entry").is_err());
}

#[test]
fn staged_quote_expansion_retains_owned_argument_cleanup_and_return_boundary() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {} struct Item {}
        impl Retire for Item {fn drop(*self) {}}
        fn make()->Item {Item {}}
        fn consume(item:Item) {}
        quote fn choose(comptime enabled:bool,item:Item)->Item {if enabled {return item;}item}
        quote fn discard(comptime enabled:bool,item:Item) {if enabled {return;}}
        fn forward(comptime enabled:bool,item:Item)->Item {choose(enabled,item)}";
    for (body,expected) in [
        ("consume(forward(true,make()));",vec!["entry","make","forward","consume","drop Item"]),
        ("discard(false,make());discard(true,make());",vec!["entry","make","drop Item","make","drop Item"]),
    ] {
        let package=compile(&format!("{prefix} fn entry() {{{body}}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn contracted_quote_preserves_snapshot_copy_permission_and_result_ownership() {
    use comptime_interpretor::{Interpreter,Value};
    let prefix="trait Retire {fn drop(*self);} trait Duplicate {} struct Item {flag:bool}
        impl Retire for Item {fn drop(*self) {}}
        fn make()->Item {Item {flag:true}}
        fn consume(item:Item) {}";
    let package=compile(&format!("{prefix}
        quote fn checked(item:Item)->result:Item where requires item.flag, ensures result.flag {{return item;}}
        fn entry() {{let item=make();assumes item.flag;consume(checked(item));}}"));
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","make","consume","drop Item"]);
    let package=compile(&format!("{prefix}
        quote fn checked(item:Item)->bool where requires item.flag, ensures item'old.flag {{true}}
        fn entry()->bool {{let item=make();assumes item.flag;checked(item)}}"));
    let failure=source_executable(&package,"entry").err().unwrap();
    assert!(format!("{failure:?}").contains("Copy"),"{failure:?}");
}

#[test]
fn contracted_quote_checked_assumption_failure_cleans_resources_once() {
    use comptime_interpretor::Interpreter;
    let package=compile("trait Retire {fn drop(*self);} trait Duplicate {}
        struct Item {flag:bool} impl Retire for Item {fn drop(*self) {}}
        fn make()->Item {Item {flag:true}}
        quote fn checked(item:Item)->result:Item where ensures not result.flag {
            assumes not item.flag;return item;
        }
        fn entry() {let item=checked(make());}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert!(vm.invoke_instance(&executable.entry,vec![]).is_err());
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","make","drop Item"]);
}

#[test]
fn effect_completion_ownership_moves_payload_and_cleans_only_remaining_owners() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        fn subject(item:Ticket)->#poll Ticket {let spare=Ticket {};item}
        fn forward(item:Ticket)->#poll Ticket {subject(item)}
        fn discard(item:Ticket)->#poll bool {subject(item);true}");
    let subject = cleanup_report(&package, "subject").unwrap();
    assert_eq!(subject.cleanup_sites.iter().map(|site|site.actions.len()).sum::<usize>(), 1, "{subject}");
    let forward = cleanup_report(&package, "forward").unwrap();
    assert!(forward.cleanup_sites.iter().all(|site|site.actions.is_empty()), "{forward}");
    let discard = cleanup_report(&package, "discard").unwrap();
    assert_eq!(discard.cleanup_sites.iter().map(|site|site.actions.len()).sum::<usize>(), 1, "{discard}");
    assert!(!discard.to_string().contains("(computation"), "{discard}");
}

#[test]
fn effect_completion_ownership_rejects_repeated_affine_delivery() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        fn subject(item:Ticket)->#poll Ticket {item}
        fn bad(item:Ticket)->#poll Ticket {subject(item);subject(item)}");
    let error = cleanup_report(&package, "bad").unwrap_err();
    assert!(error.reason.contains("available") || error.reason.contains("move"), "{error}");
}

#[test]
fn effect_completion_ownership_checks_early_returns_and_branch_moves() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        fn subject(item:Ticket, flag:bool)->#poll Ticket {
            let spare=Ticket {};
            if flag {return item;}
            item
        }");
    let report = cleanup_report(&package, "subject").unwrap();
    let drops = report.cleanup_sites.iter().flat_map(|site| &site.actions).filter(|action| {
        matches!(action, elaboration::verification::moves::CleanupAction::Planned { plan, .. }
            if plan.steps.iter().any(|step| matches!(step, elaboration::verification::drop::CleanupStep::Invoke { .. })))
    }).count();
    assert_eq!(drops, 2, "{report}");
}

#[test]
fn effect_completion_ownership_retains_callable_provenance() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        typealias Callback=%{ticket:Ticket} fn();
        fn build()->#poll Callback {let ticket=Ticket {};|| {let consumed=ticket;}}
        fn good()->#poll bool {let callback=build();callback();true}
        fn bad()->#poll bool {let callback=build();callback();callback();true}");
    let good = cleanup_report(&package, "good").unwrap();
    assert!(good.deferred_calls.is_empty(), "{good}");
    assert!(good.closures.iter().any(|closure|closure.invoked), "{good}");
    let error = cleanup_report(&package, "bad").unwrap_err();
    assert!(error.reason.contains("available") || error.reason.contains("move") || error.reason.contains("consumed"), "{error}");
}

#[test]
fn effect_completion_ownership_rejects_escaping_local_pointer() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        fn bad()->#poll *Ticket {let ticket=Ticket {};ticket.ref}");
    let error = cleanup_report(&package, "bad").unwrap_err();
    assert!(error.reason.contains("scope") || error.reason.contains("local") || error.reason.contains("retir"), "{error}");
}

#[test]
fn effect_completion_ownership_checks_quote_expansion_without_copying_owners() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        quote fn forward(item:Ticket)->#poll Ticket {return item;}
        fn good(item:Ticket)->#poll Ticket {forward(item)}
        fn bad(item:Ticket)->#poll Ticket {forward(item);forward(item)}");
    let report = cleanup_report(&package, "good").unwrap();
    assert!(report.copies.is_empty(), "{report}");
    let error = cleanup_report(&package, "bad").unwrap_err();
    assert!(error.reason.contains("available") || error.reason.contains("move"), "{error}");
}

#[test]
fn effect_escape_checkpoint_excludes_transferred_inputs_and_preserves_resume_state() {
    use elaboration::verification::moves::{CleanupAction, CleanupReason};
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect send(item:Ticket)->bool;
        fn subject(sent:Ticket, kept:Ticket)->#send bool {send(sent)#}");
    let function = package.functions.iter().find(|f|f.name=="subject").unwrap();
    let report = cleanup_report(&package, "subject").unwrap();
    let exits:Vec<_> = report.cleanup_sites.iter().filter(|s|s.reason==CleanupReason::EffectEscape).collect();
    assert_eq!(exits.len(), 1, "{report}");
    assert_eq!(exits[0].actions.len(), 1, "{report}");
    let CleanupAction::Planned {candidate, ..} = &exits[0].actions[0] else {panic!("{report}")};
    let CleanupAction::Owned {remaining, ..} = candidate.as_ref() else {panic!("{report}")};
    assert_eq!(remaining.place.root.local, function.parameters[1].index, "{report}");
    assert_eq!(remaining.place.root.owner, u64::from(function.parameters[1].owner), "{report}");
    assert_eq!(report.exits.len(), 1, "escape must not become a normal return: {report}");
    assert!(report.cleanup_sites.iter().any(|s|s.reason==CleanupReason::FunctionEnd && s.actions.len()==1), "{report}");
    assert!(report.to_string().contains("(reason effect-escape)"));
    assert_eq!(iir::cleanup::Boundary::EffectEscape.to_string(), "effect-escape");
}

#[test]
fn effect_escape_checkpoint_keeps_enclosing_partial_construction_in_order() {
    use elaboration::verification::moves::{CleanupAction, CleanupReason};
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        fn subject()->#poll bool {let older=Ticket {};let pair=(Ticket {},poll()#);true}");
    let report = cleanup_report(&package, "subject").unwrap();
    let checkpoint = report.cleanup_sites.iter().find(|s|s.reason==CleanupReason::EffectEscape).unwrap();
    assert_eq!(checkpoint.actions.len(),2,"{report}");
    assert!(matches!(&checkpoint.actions[0], CleanupAction::Planned {candidate,..} if matches!(candidate.as_ref(),CleanupAction::Prepared {value,..} if value.slot==0)),"{report}");
    assert!(matches!(&checkpoint.actions[1], CleanupAction::Planned {candidate,..} if matches!(candidate.as_ref(),CleanupAction::Owned {..})),"{report}");
}

#[test]
fn effect_escape_checkpoint_is_propagated_to_effectful_callers() {
    use elaboration::verification::moves::CleanupReason;
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        fn child()->#poll bool {poll()#}
        fn parent()->#poll bool {let kept=Ticket {};child()}");
    let report = cleanup_report(&package,"parent").unwrap();
    let checkpoint = report.cleanup_sites.iter().find(|s|s.reason==CleanupReason::EffectEscape).unwrap();
    assert_eq!(checkpoint.actions.len(),1,"{report}");
    assert_eq!(report.exits.len(),1,"{report}");
}

#[test]
fn effect_escape_checkpoint_preparation_failure_keeps_request_unentered() {
    use elaboration::verification::moves::{CleanupAction,CleanupReason};
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect send(first:Ticket,second:Ticket)->bool;
        fn subject()->#send bool {send(Ticket {},do {assumes false;Ticket {}})#}");
    let report = cleanup_report(&package,"subject").unwrap();
    assert!(!report.cleanup_sites.iter().any(|s|s.reason==CleanupReason::EffectEscape),"{report}");
    assert!(report.cleanup_sites.iter().filter(|s|s.reason==CleanupReason::PredicateFailure).flat_map(|s|&s.actions)
        .any(|a|matches!(a,CleanupAction::Planned {candidate,..} if matches!(candidate.as_ref(),CleanupAction::Prepared {value,..} if value.slot==0))),"{report}");
}

#[test]
fn effect_escape_checkpoint_rejects_repeated_affine_request_inputs() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect send(item:Ticket)->bool;
        fn subject(item:Ticket)->#send bool {send(item)#;send(item)#}");
    let error = cleanup_report(&package,"subject").unwrap_err();
    assert!(error.reason.contains("available") || error.reason.contains("move"),"{error}");
}

#[test]
fn effect_escape_checkpoint_argument_return_uses_lexical_return_cleanup() {
    use elaboration::verification::moves::{CleanupAction,CleanupReason};
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect send(first:Ticket,second:Ticket)->bool;
        fn subject()->#send bool {send(Ticket {},do {return true;})#}");
    let report = cleanup_report(&package,"subject").unwrap();
    assert!(!report.cleanup_sites.iter().any(|s|s.reason==CleanupReason::EffectEscape),"{report}");
    assert!(report.cleanup_sites.iter().filter(|s|s.reason==CleanupReason::Return).flat_map(|s|&s.actions)
        .any(|a|matches!(a,CleanupAction::Planned {candidate,..} if matches!(candidate.as_ref(),CleanupAction::Prepared {value,..} if value.slot==0))),"{report}");
}

#[test]
fn effect_escape_checkpoint_does_not_consume_resources_on_resume() {
    let package = compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        effect poll()->bool;
        fn child()->#poll bool {poll()#}
        fn consume(item:Ticket)->bool {true}
        fn subject(item:Ticket)->#poll bool {child();consume(item)}");
    let report = cleanup_report(&package,"subject").unwrap();
    assert_eq!(report.exits.len(),1,"{report}");
    assert!(report.cleanup_sites.iter().any(|s|s.reason==elaboration::verification::moves::CleanupReason::EffectEscape),"{report}");
    assert!(report.cleanup_sites.iter().filter(|s|s.reason==elaboration::verification::moves::CleanupReason::FunctionEnd)
        .all(|s|s.actions.is_empty()),"{report}");
}

#[test]
fn effect_escape_checkpoint_uses_suspended_capture_state_not_normal_return_state() {
    use elaboration::verification::{moves::CleanupReason, drop::CleanupStep, moves::CleanupAction};
    for (body, retained) in [("poll()#;consume(ticket)",true), ("consume(ticket);poll()#",false)] {
        let package = compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            effect poll()->bool;
            typealias Callback=%{{ticket:Ticket}} fn()->#poll bool;
            fn consume(ticket:Ticket)->bool {{true}}
            fn subject()->#poll bool {{
                let ticket=Ticket {{}};
                let callback:Callback=|| {{{body}}};
                callback()
            }}"));
        let report = cleanup_report(&package,"subject").unwrap();
        let checkpoints:Vec<_> = report.cleanup_sites.iter().filter(|s|s.reason==CleanupReason::EffectEscape).collect();
        let drops = checkpoints.iter().flat_map(|s|&s.actions).filter(|a|matches!(a,
            CleanupAction::Planned {plan,..} if plan.steps.iter().any(|s|matches!(s,CleanupStep::Invoke {..})))).count();
        assert_eq!(drops,usize::from(retained),"{body}: {report}");
        assert_eq!(report.effect_exits.len(),1,"{body}: {report}");
        assert_eq!(report.exits.len(),1,"{body}: {report}");
    }
}

#[test]
fn scheduled_direct_handler_cleans_callee_and_handler_locals_on_resume() {
    use comptime_interpretor::{Interpreter,Value};
    for handler in ["do {let local=Inside {};true}","do {let local=Inside {};resume true;}"] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Outside {{}} struct Inside {{}}
            impl Retire for Outside {{fn drop(*self) {{}}}}
            impl Retire for Inside {{fn drop(*self) {{}}}}
            effect poll()->bool;
            fn subject()->#poll bool {{let held=Outside {{}};poll()#}}
            fn entry()->bool {{subject()# {{poll()=>{handler},}}}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),
            ["entry","subject","drop Inside","drop Outside"]);
    }
}

#[test]
fn scheduled_direct_handler_transfers_affine_result_and_cleans_pattern_residual() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Output {} struct Ignored {} struct Local {} struct Frame {}
        impl Retire for Output {fn drop(*self) {}} impl Retire for Ignored {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}} impl Retire for Frame {fn drop(*self) {}}
        effect send(first:Output,second:Ignored)->Output;
        fn subject()->#send Output {let frame=Frame {};send(Output {},Ignored {})#}
        fn entry() {subject()# {send(first,_)=>do {let local=Local {};resume first;},};}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","subject","drop Local","drop Ignored","drop Frame","drop Output"]);
}

#[test]
fn scheduled_direct_handler_repeated_requests_get_independent_local_owners() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        effect poll()->bool;
        fn subject()->#poll bool {poll()# and poll()#}
        fn entry()->bool {subject()# {poll()=>do {let item=Item {};true},}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Item","drop Item"]);
}

#[test]
fn scheduled_direct_handler_failed_guard_does_not_consume_operation_inputs() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        effect send(item:Item)->bool;
        fn subject()->#send bool {send(Item {})#}
        fn entry()->bool {subject()# {
            send(item) if false=>false,
            send(item)=>do {let owned=item;true},
        }}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Item"]);
}

#[test]
fn scheduled_direct_handler_rejection_unwinds_handler_subject_and_caller_once() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Input {} struct Local {} struct Subject {} struct Caller {}
        impl Retire for Input {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}}
        impl Retire for Subject {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}}
        effect send(item:Input)->bool;
        fn subject()->#send bool {let frame=Subject {};send(Input {})#}
        fn entry()->bool {let caller=Caller {};subject()# {send(item)=>do {let local=Local {};assumes false;true},}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","subject","drop Local","drop Input","drop Subject","drop Caller"]);
}

#[test]
fn scheduled_direct_handler_keeps_unimplemented_affine_capture_gate() {
    for body in ["let local=outer;true"] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{}} impl Retire for Held {{fn drop(*self) {{}}}}
            effect poll()->bool;fn subject()->#poll bool {{poll()#}}
            fn entry()->bool {{let outer=Held {{}};subject()# {{poll()=>do {{{body}}},}}}}"));
        let error=source_executable(&package,"entry").unwrap_err();
        assert!(error.contains("lexical") || error.contains("request-time"),"{error}");
    }
}

#[test]
fn scheduled_direct_handler_nested_forwarding_retains_input_until_selected_arm() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        effect send(item:Item,flag:bool)->bool;
        fn subject()->#send bool {send(Item {},false)#}
        fn middle()->#send bool {subject()# {send(item,true)=>true,}}
        fn entry()->bool {middle()# {send(item,flag)=>flag,}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(false));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","middle","subject","drop Item"]);
}

#[test]
fn scheduled_direct_handler_nested_closure_can_own_an_operation_input() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Item {} impl Retire for Item {fn drop(*self) {}}
        effect send(item:Item)->bool;
        fn subject()->#send bool {send(Item {})#}
        fn entry()->bool {subject()# {send(item)=>do {let callback=|| {let owned=item;return true;};callback()},}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Item"]);
}

#[test]
fn scheduled_direct_handler_failure_cleans_call_argument_preparation_in_order() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Prepared {} struct Caller {} struct Handler {}
        impl Retire for Prepared {fn drop(*self) {}} impl Retire for Caller {fn drop(*self) {}}
        impl Retire for Handler {fn drop(*self) {}}
        effect poll()->bool;
        fn subject()->#poll bool {poll()#}
        fn target(item:Prepared,value:bool)->bool {value}
        fn entry()->bool {let caller=Caller {};target(Prepared {},subject()# {poll()=>do {let local=Handler {};assumes false;true},})}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Handler","drop Prepared","drop Caller"]);
}

#[test]
fn scheduled_direct_handler_quote_expansion_keeps_caller_and_handler_cleanup_separate() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Frame {} struct Local {}
        impl Retire for Frame {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}}
        effect poll()->bool;
        quote fn subject(flag:bool)->#poll bool {let frame=Frame {};if flag {return poll()#;}false}
        fn entry()->bool {subject(true)# {poll()=>do {let local=Local {};resume true;},}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Local","drop Frame"]);
}

#[test]
fn scheduled_direct_handler_missing_context_rejects_before_guard_execution() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        effect poll()->bool;
        fn guard()->bool {true}
        fn subject()->#poll bool {poll()#}
        fn entry()->bool {subject()# {poll() if guard()=>true,poll()=>false,}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut schedule=executable.schedule.clone();
    schedule.functions.retain(|function|!function.context.iter().any(|context|
        matches!(context,iir::cleanup::ContextKey::EffectHandler {..})));
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::InvalidCallPlan);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject"]);
}

#[test]
fn scheduled_handler_scalar_capture_observes_subject_argument_writes() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        effect poll()->bool;
        fn subject(input:bool)->#poll bool {poll()#}
        fn entry()->bool {var flag=false;subject(do {flag=true;true})# {poll()=>flag,}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
}

#[test]
fn scheduled_handler_scalar_capture_writes_are_shared_across_requests() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Local {} impl Retire for Local {fn drop(*self) {}}
        effect poll()->bool;
        fn subject()->#poll bool {let first=poll()#;let second=poll()#;first and not second}
        fn entry()->bool {var flag=false;let result=subject()# {poll()=>do {let local=Local {};flag=not flag;resume flag;},};result and not flag}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Local","drop Local"]);
}

#[test]
fn scheduled_handler_scalar_capture_rejects_address_exposure_before_or_during_subject() {
    for (before,argument,handler) in [
        ("let pointer=flag.ref;","true","flag"),
        ("","do {let pointer=flag.ref;true}","flag"),
        ("","true","do {let pointer=flag.ref;flag}"),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            effect poll()->bool;fn subject(input:bool)->#poll bool {{poll()#}}
            fn entry()->bool {{var flag=false;{before} subject({argument})# {{poll()=>{handler},}}}}"));
        let error=source_executable(&package,"entry").unwrap_err();
        assert!(error.contains("memory contract"),"{error}");
    }
}

#[test]
fn scheduled_handler_scalar_capture_rejection_does_not_drop_installer_storage_twice() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Local {}
        impl Retire for Caller {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}}
        effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn entry()->bool {let caller=Caller {};var flag=false;subject()# {
            poll()=>do {let local=Local {};flag=true;assumes false;flag},
        }}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Local","drop Caller"]);
}

#[test]
fn scheduled_handler_scalar_capture_nested_installations_share_the_correct_scope() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        effect poll()->bool;effect ask(value:bool)->bool;
        fn inner()->#poll bool {poll()#}
        fn subject()->#ask bool {ask(false)#}
        fn entry()->bool {subject()# {ask(value)=>do {
            var flag=value;
            let result=inner()# {poll()=>do {flag=not flag;resume flag;},};
            result and flag
        },}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","inner"]);
}

#[test]
fn scheduled_scalar_passing_provider_runs_once_before_arguments_and_cleans_locals() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Local {} impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect config()->bool;
        fn provider()->bool {let local=Local {};true}
        fn argument()->bool {false}
        fn subject(value:bool)->#config bool {config()# and config()# and not value}
        fn entry()->bool {subject(argument())# {config()=>provider(),}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","provider","drop Local","argument","subject"]);
}

#[test]
fn scheduled_scalar_passing_provider_failure_cleans_installer_and_prepared_arguments() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Prepared {} struct Local {}
        impl Retire for Caller {fn drop(*self) {}} impl Retire for Prepared {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect config()->bool;
        fn provider()->bool {let local=Local {};assumes false;true}
        fn argument()->bool {true}
        fn subject(value:bool)->#config bool {config()#}
        fn target(item:Prepared,value:bool)->bool {value}
        fn entry()->bool {let caller=Caller {};target(Prepared {},subject(argument())# {config()=>provider(),})}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","provider","drop Local","drop Prepared","drop Caller"]);
}

#[test]
fn scheduled_scalar_passing_slot_keeps_provider_snapshot_after_argument_writes() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Local {} impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect config()->bool;
        quote fn subject(value:bool)->#config bool {config()# and config()#}
        fn entry()->bool {var flag=false;
            let result=subject(do {flag=false;true})# {config()=>do {let local=Local {};flag=true;resume flag;},};
            result and not flag
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Local"]);
}

#[test]
fn scheduled_scalar_passing_unused_slot_still_runs_and_cleans_provider_once() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Local {} impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect config()->bool;
        fn provider()->bool {let local=Local {};true}
        fn subject()->#config bool {false}
        fn entry()->bool {subject()# {config()=>provider(),}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(false));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","provider","drop Local","subject"]);
}

#[test]
fn scheduled_scalar_passing_later_provider_failure_stops_installation_before_subject() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct First {} struct Second {} struct Caller {}
        impl Retire for First {fn drop(*self) {}} impl Retire for Second {fn drop(*self) {}}
        impl Retire for Caller {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect first()->bool;
        ^attrs.effects.direct_passing effect second()->bool;
        fn provide_first()->bool {let local=First {};true}
        fn provide_second()->bool {let local=Second {};assumes false;true}
        fn subject()->#[first,second] bool {first()# and second()#}
        fn entry()->bool {let caller=Caller {};subject()# {first()=>provide_first(),second()=>provide_second(),}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),
        ["entry","provide_first","drop First","provide_second","drop Second","drop Caller"]);
}

#[test]
fn scheduled_scalar_passing_partial_provider_uses_outer_slot_without_reinvoking_provider() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Local {} impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect config()->bool;
        fn provider()->bool {let local=Local {};true}
        fn subject()->#config bool {config()# and config()#}
        fn middle(flag:bool)->#config bool {subject()# {config() if flag=>false,}}
        fn entry()->bool {middle(false)# {config()=>provider(),}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","provider","drop Local","middle","subject"]);
}

#[test]
fn scheduled_scalar_passing_void_provider_runs_once_for_repeated_requests() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Local {} impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect setup();
        fn subject()->#setup bool {setup()#;setup()#;true}
        fn entry()->bool {var ready=false;
            let result=subject()# {setup()=>do {let local=Local {};ready=true;},};
            result and ready
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Local","subject"]);
}

#[test]
fn scheduled_handler_scalar_capture_closure_addresses_private_storage() {
    use comptime_interpretor::{Interpreter,Value};
    for body in [
        "let callback=|| {let pointer=flag.ref;pointer.* = true;flag};let result=callback();result and not flag",
        "let factory=|| {|| {let pointer=flag.ref;pointer.* = true;flag}};let callback=factory();let result=callback();result and not flag",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            effect poll()->bool;fn subject()->#poll bool {{poll()#}}
            fn entry()->bool {{var flag=false;subject()# {{poll()=>do {{{body}}},}}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{body}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{body}");
    }
}

#[test]
fn scheduled_handler_lexical_return_unwinds_handler_subject_and_installer() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Subject {} struct Local {}
        impl Retire for Caller {fn drop(*self) {}} impl Retire for Subject {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        effect poll()->bool;
        fn subject()->#poll bool {let subject=Subject {};poll()#;assumes false;false}
        fn entry()->bool {let caller=Caller {};subject()# {poll()=>do {let local=Local {};return true;},};assumes false;false}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Local","drop Subject","drop Caller"]);
}

#[test]
fn scheduled_handler_lexical_return_preserves_owned_result_and_prepared_arguments() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Prepared {} struct Subject {} struct Local {} struct Output {}
        impl Retire for Caller {fn drop(*self) {}} impl Retire for Prepared {fn drop(*self) {}}
        impl Retire for Subject {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}}
        impl Retire for Output {fn drop(*self) {}}
        effect poll()->bool;
        fn subject()->#poll bool {let subject=Subject {};poll()#}
        fn target(prepared:Prepared,flag:bool) {assumes false;}
        fn build()->Output {let caller=Caller {};target(Prepared {},subject()# {
            poll()=>do {let local=Local {};return Output {};},
        });Output {}}
        fn entry()->bool {let output=build();true}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","build","subject","drop Local","drop Subject","drop Prepared","drop Caller","drop Output"]);
}

#[test]
fn scheduled_handler_lexical_return_checks_installer_postcondition_and_owned_result() {
    use comptime_interpretor::{Interpreter,Value,ErrorKind};
    for reject in [false,true] {
        let mut package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
            struct Caller {} struct Subject {} struct Local {} struct Output {}
            impl Retire for Caller {fn drop(*self) {}} impl Retire for Subject {fn drop(*self) {}}
            impl Retire for Local {fn drop(*self) {}} impl Retire for Output {fn drop(*self) {}}
            effect poll()->bool;fn subject()->#poll bool {let subject=Subject {};poll()#}
            fn build()->result:Output where ensures true {let caller=Caller {};subject()# {
                poll()=>do {let local=Local {};return Output {};},
            };Output {}}
            fn entry()->bool {let output=build();true}");
        if reject {
            let build=package.functions.iter_mut().find(|f|f.name=="build").unwrap();
            build.postconditions[0].kind=nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
        }
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        let result=vm.invoke_instance(&executable.entry,vec![]);
        if reject {
            assert_eq!(result.unwrap_err().kind,ErrorKind::ProgramContractRejected);
            assert_eq!(executed_names(&package,&executable,vm.events()),["entry","build","subject","drop Local","drop Subject","drop Output","drop Caller"]);
        } else {
            assert_eq!(result.unwrap(),Value::Bool(true));
            assert_eq!(executed_names(&package,&executable,vm.events()),["entry","build","subject","drop Local","drop Subject","drop Caller","drop Output"]);
        }
    }
}

#[test]
fn scheduled_handler_lexical_return_provider_exits_before_subject_arguments() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Local {} impl Retire for Caller {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect config()->bool;
        fn argument()->bool {assumes false;false}
        fn subject(flag:bool)->#config bool {config()#}
        fn entry()->bool {let caller=Caller {};subject(argument())# {
            config()=>do {let local=Local {};return true;},
        }}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Local","drop Caller"]);
}

#[test]
fn scheduled_handler_lexical_return_crosses_nested_handler_scopes() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Outer {} struct Inner {} struct Subject {}
        impl Retire for Caller {fn drop(*self) {}} impl Retire for Outer {fn drop(*self) {}}
        impl Retire for Inner {fn drop(*self) {}} impl Retire for Subject {fn drop(*self) {}}
        effect ask()->bool;effect poll()->bool;
        fn outer()->#ask bool {let subject=Subject {};ask()#}
        fn inner()->#poll bool {poll()#}
        fn entry()->bool {let caller=Caller {};outer()# {ask()=>do {
            let outer=Outer {};inner()# {poll()=>do {let inner=Inner {};return true;},}
        },}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","outer","inner","drop Inner","drop Outer","drop Subject","drop Caller"]);
}

#[test]
fn scheduled_handler_lexical_return_respects_closure_and_quote_call_boundaries() {
    use comptime_interpretor::{Interpreter,Value};
    for source in [
        // A handler installed by the callback returns from that callback only.
        "fn subject()->#poll bool {poll()#} fn entry()->bool {let caller=Caller {};let callback=|| {let local=Local {};subject()# {poll()=>do {return true;},};false};let value=callback();value}",
        // Quote expansion has no subject invocation frame to unwind.
        "quote fn subject()->#poll bool {let local=Local {};poll()#;false} fn entry()->bool {let caller=Caller {};subject()# {poll()=>do {return true;},};false}",
        // Partial forwarding must retain the outer return target.
        "fn subject()->#poll bool {let local=Local {};poll()#} fn middle()->#poll bool {subject()# {poll() if false=>false,}} fn entry()->bool {let caller=Caller {};middle()# {poll()=>do {return true;},};false}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Caller {{}} struct Local {{}} impl Retire for Caller {{fn drop(*self) {{}}}}
            impl Retire for Local {{fn drop(*self) {{}}}} effect poll()->bool;{source}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{source}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{source}");
        let names=executed_names(&package,&executable,vm.events());
        let drops:Vec<_>=names.iter().filter(|name| name.starts_with("drop ")).cloned().collect();
        assert_eq!(drops,["drop Local","drop Caller"],"{source}");
    }
}

#[test]
fn scheduled_handler_lexical_return_unnamed_result_rejection_retires_payload() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let mut package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Output {} impl Retire for Caller {fn drop(*self) {}}
        impl Retire for Output {fn drop(*self) {}} effect poll()->bool;
        fn subject()->#poll bool {poll()#}
        fn entry()->Output where ensures true {let caller=Caller {};subject()# {poll()=>do {return Output {};},};Output {}}");
    let entry=package.functions.iter_mut().find(|f|f.name=="entry").unwrap();
    entry.postconditions[0].kind=nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Output","drop Caller"]);
}

#[test]
fn scheduled_handler_lexical_return_does_not_relax_local_pointer_escape_checks() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        effect poll()->*bool;fn subject()->#poll *bool {poll()#}
        fn entry()->*bool {subject()# {poll()=>do {let local=true;return local.ref;},}}");
    let error=source_executable(&package,"entry").unwrap_err();
    assert!(error.contains("escaping provenance"),"{error}");
    assert_eq!(iir::cleanup::Boundary::EffectReturn.to_string(),"effect-return");
    assert_eq!(elaboration::verification::moves::CleanupReason::EffectReturn.to_string(),"effect-return");
}

#[test]
fn scheduled_handler_lexical_return_conditional_exit_keeps_normal_resume_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Input {} struct Local {}
        impl Retire for Caller {fn drop(*self) {}} impl Retire for Input {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        effect poll()->bool;fn subject(input:Input)->#poll bool {poll()#}
        fn entry(stop:bool)->bool {let caller=Caller {};let input=Input {};subject(input)# {
            poll()=>do {let local=Local {};if stop {return true;} resume false;},
        };false}");
    let executable=source_executable(&package,"entry").unwrap();
    for stop in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(stop)]).unwrap(),Value::Bool(stop));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Local","drop Input","drop Caller"]);
    }
}

#[test]
fn scheduled_handler_loop_exit_break_preserves_owners_outside_loop() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Outer {} struct Iteration {} struct Subject {} struct Local {}
        impl Retire for Outer {fn drop(*self) {}} impl Retire for Iteration {fn drop(*self) {}}
        impl Retire for Subject {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}}
        effect poll()->bool;fn subject()->#poll bool {let subject=Subject {};poll()#;assumes false;false}
        fn entry()->bool {let outer=Outer {};while true {let iteration=Iteration {};subject()# {
            poll()=>do {let local=Local {};break;},
        };assumes false;}true}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Local","drop Subject","drop Iteration","drop Outer"]);
}

#[test]
fn scheduled_handler_loop_exit_continue_reenters_loop_with_shared_capture_updates() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Outer {} struct Iteration {} struct Subject {} struct Local {}
        impl Retire for Outer {fn drop(*self) {}} impl Retire for Iteration {fn drop(*self) {}}
        impl Retire for Subject {fn drop(*self) {}} impl Retire for Local {fn drop(*self) {}}
        effect poll()->bool;fn subject()->#poll bool {let subject=Subject {};poll()#}
        fn entry()->bool {let outer=Outer {};var first=true;while true {
            let iteration=Iteration {};subject()# {poll()=>do {
                let local=Local {};if first {first=false;continue;}break;
            },};assumes false;
        }not first}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Local","drop Subject","drop Iteration","subject","drop Local","drop Subject","drop Iteration","drop Outer"]);
}

#[test]
fn scheduled_handler_loop_exit_cleans_prepared_arguments_but_preserves_enclosing_owner() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Outer {} struct Prepared {} struct Iteration {} struct Input {}
        impl Retire for Outer {fn drop(*self) {}} impl Retire for Prepared {fn drop(*self) {}}
        impl Retire for Iteration {fn drop(*self) {}} impl Retire for Input {fn drop(*self) {}}
        effect poll(input:Input)->bool;fn subject()->#poll bool {poll(Input {})#}
        fn target(prepared:Prepared,flag:bool) {assumes false;}
        fn entry()->bool {let outer=Outer {};while true {let iteration=Iteration {};
            target(Prepared {},subject()# {poll(input)=>do {break;},});assumes false;
        }true}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Input","drop Prepared","drop Iteration","drop Outer"]);
}

#[test]
fn scheduled_handler_loop_exit_provider_runs_before_arguments() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Outer {} struct Iteration {} struct Local {}
        impl Retire for Outer {fn drop(*self) {}} impl Retire for Iteration {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        ^attrs.effects.direct_passing effect config()->bool;
        fn argument()->bool {assumes false;false}
        fn subject(flag:bool)->#config bool {config()#}
        fn entry()->bool {let outer=Outer {};var first=true;while true {let iteration=Iteration {};
            subject(argument())# {config()=>do {let local=Local {};if first {first=false;continue;}break;},};
            assumes false;
        }not first}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Local","drop Iteration","drop Local","drop Iteration","drop Outer"]);
}

#[test]
fn scheduled_handler_loop_exit_target_inside_an_outer_handler_keeps_that_handler_live() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Outer {} struct Iteration {} struct Local {}
        impl Retire for Outer {fn drop(*self) {}} impl Retire for Iteration {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        effect ask()->bool;effect poll()->bool;
        fn outer()->#ask bool {ask()#} fn inner()->#poll bool {poll()#}
        fn entry()->bool {outer()# {ask()=>do {
            let outer=Outer {};var first=true;while true {let iteration=Iteration {};
                inner()# {poll()=>do {let local=Local {};if first {first=false;continue;}break;},};assumes false;
            }resume not first;
        },}}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","outer","inner","drop Local","drop Iteration","inner","drop Local","drop Iteration","drop Outer"]);
}

#[test]
fn scheduled_handler_loop_exit_crosses_nested_handlers_to_installer_loop() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Iteration {} struct Outer {} struct Inner {} struct Subject {}
        impl Retire for Iteration {fn drop(*self) {}} impl Retire for Outer {fn drop(*self) {}}
        impl Retire for Inner {fn drop(*self) {}} impl Retire for Subject {fn drop(*self) {}}
        effect ask()->bool;effect poll()->bool;
        fn outer()->#ask bool {let subject=Subject {};ask()#} fn inner()->#poll bool {poll()#}
        fn entry()->bool {while true {let iteration=Iteration {};outer()# {ask()=>do {
            let outer=Outer {};inner()# {poll()=>do {let inner=Inner {};break;},}
        },};assumes false;}true}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","outer","inner","drop Inner","drop Outer","drop Subject","drop Iteration"]);
}

#[test]
fn scheduled_handler_loop_exit_labels_select_distinct_cleanup_boundaries() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Outer {} struct Inner {} impl Retire for Outer {fn drop(*self) {}}
        impl Retire for Inner {fn drop(*self) {}}
        effect poll()->bool;fn subject()->#poll bool {poll()#} fn after() {}
        fn entry(flag:bool)->bool {while:outside true {let outer=Outer {};while:inside true {let inner=Inner {};
            subject()# {poll()=>do {if flag {break outside;}break inside;},};assumes false;
        }after();break;}true}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
        let expected=if flag {vec!["entry","subject","drop Inner","drop Outer"]}
            else {vec!["entry","subject","drop Inner","after","drop Outer"]};
        assert_eq!(executed_names(&package,&executable,vm.events()),expected);
    }
}

#[test]
fn scheduled_handler_loop_exit_continue_cannot_resurrect_transferred_owner() {
    use comptime_interpretor::{Interpreter,Value};
    for control in ["break","continue"] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            effect poll()->bool;fn subject(ticket:Ticket)->#poll bool {{poll()#}}
            fn entry() {{let ticket=Ticket {{}};while true {{subject(ticket)# {{poll()=>do {{{control};}},}};break;}}}}"));
        let executable=source_executable(&package,"entry");
        if control=="continue" {
            assert!(executable.is_err(),"the handler back edge must not restore the transferred ticket");
        } else {
            let executable=executable.unwrap();
            let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                .with_cleanup_schedule(&executable.schedule).unwrap();
            assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Unit);
            assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Ticket"]);
        }
    }
}

#[test]
fn scheduled_handler_loop_exit_quote_subject_and_forwarded_request() {
    use comptime_interpretor::{Interpreter,Value};
    for subject in [
        "quote fn subject()->#poll bool {let local=Local {};poll()#}",
        "fn leaf()->#poll bool {let local=Local {};poll()#} fn subject()->#poll bool {leaf()# {poll() if false=>false,}}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Local {{}} impl Retire for Local {{fn drop(*self) {{}}}}
            effect poll()->bool;{subject}
            fn entry()->bool {{while true {{subject()# {{poll()=>do {{break;}},}};assumes false;}}true}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|name|name.as_str()=="drop Local").count(),1,"{names:?}");
    }
}

#[test]
fn scheduled_handler_loop_exit_uses_suspended_closure_capture_state_for_back_edges() {
    use comptime_interpretor::{Interpreter,Value};
    for (body, retained) in [("poll()#;consume(ticket)",true),("consume(ticket);poll()#",false)] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            effect poll()->bool;typealias Callback=%{{ticket:Ticket}} fn()->#poll bool;
            fn consume(ticket:Ticket)->bool {{true}}
            fn entry()->bool {{let ticket=Ticket {{}};let callback:Callback=|| {{{body}}};var first=true;
                while true {{callback()# {{poll()=>do {{if first {{first=false;continue;}}break;}},}};break;}}
                not first
            }}"));
        let executable=source_executable(&package,"entry");
        if !retained {
            assert!(executable.is_err(),"continue must retain the move performed before the request");
            continue;
        }
        let executable=executable.unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Ticket"]);
    }
}

#[test]
fn scheduled_handler_loop_exit_preserves_argument_preparation_outside_target_loop() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Prepared {} struct Iteration {}
        impl Retire for Prepared {fn drop(*self) {}} impl Retire for Iteration {fn drop(*self) {}}
        effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn target(prepared:Prepared,flag:bool)->bool {flag}
        fn entry()->bool {target(Prepared {},do {var first=true;while true {let iteration=Iteration {};
            subject()# {poll()=>do {if first {first=false;continue;}break;},};assumes false;
        }not first})}");
    let executable=source_executable(&package,"entry").unwrap();
    let dump=executable.schedule.to_string();
    assert!(dump.contains("(effect-break (loop"),"{dump}");
    assert!(dump.contains("(effect-continue (loop"),"{dump}");
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","subject","drop Iteration","subject","drop Iteration","target","drop Prepared"]);
}

#[test]
fn scheduled_handler_returned_callable_delivers_owned_environment() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} struct Local {} impl Retire for Ticket {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        typealias Callback=%{ticket:Ticket} fn()->Ticket;
        effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn build()->Callback {subject()# {poll()=>do {let local=Local {};let ticket=Ticket {};return || ticket;},};let ticket=Ticket {};|| ticket}
        fn entry()->bool {let callback=build();let ticket=callback();true}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","build","subject","drop Local","drop Ticket"]);
}

#[test]
fn scheduled_handler_returned_callable_preserves_conditional_code_candidates() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Callback=%{value:bool} fn(bool)->bool;
        effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn build(flag:bool)->result:Callback where ensures true {subject()# {poll()=>do {
            if flag {let left=true;return |value:bool| value and left;}
            let right=true;return |value:bool| not value and right;
        },};let other=false;|value:bool| value and other}
        fn entry(flag:bool)->bool {let callback=build(flag);let first=callback(true);let second=callback(false);first==flag and second==not flag}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
    }
}

#[test]
fn scheduled_handler_returned_callable_aggregate_and_enum_payloads_keep_evidence() {
    use comptime_interpretor::{Interpreter,Value};
    for source in [
        "struct Bundle {action:Callback} fn build()->Bundle {subject()# {poll()=>do {let ticket=Ticket {};return Bundle {action:|| ticket};},};let ticket=Ticket {};Bundle {action:|| ticket}} fn entry()->bool {let bundle=build();let ticket=bundle.action();true}",
        "enum Choice {none,some(Callback)} fn build()->Choice {subject()# {poll()=>do {let ticket=Ticket {};return Choice.some(|| ticket);},};Choice.none} fn entry()->bool {build() match {.none=>false,.some(action)=>do {let ticket=action();true}}}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            typealias Callback=%{{ticket:Ticket}} fn()->Ticket;
            effect poll()->bool;fn subject()->#poll bool {{poll()#}} {source}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{source}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","build","subject","drop Ticket"]);
    }
}

#[test]
fn scheduled_handler_returned_callable_nested_and_provider_returns_preserve_context() {
    use comptime_interpretor::{Interpreter,Value};
    for source in [
        "effect ask()->bool;fn outer()->#ask bool {ask()#} fn build()->Callback {outer()# {ask()=>do {subject()# {poll()=>do {let ticket=Ticket {};return || ticket;},}},};let ticket=Ticket {};|| ticket}",
        "^attrs.effects.direct_passing effect config()->bool;fn configured()->#config bool {config()#} fn build()->Callback {configured()# {config()=>do {let ticket=Ticket {};return || ticket;},};let ticket=Ticket {};|| ticket}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            typealias Callback=%{{ticket:Ticket}} fn()->Ticket;
            effect poll()->bool;fn subject()->#poll bool {{poll()#}} {source}
            fn entry()->bool {{let callback=build();let ticket=callback();true}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{source}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.iter().filter(|name|name.as_str()=="drop Ticket").count(),1,"{names:?}");
        assert!(!names.iter().any(|name|name=="configured"),"{names:?}");
    }
}

#[test]
fn scheduled_handler_returned_callable_postcondition_failure_retires_environment() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    for result in ["Callback","result:Callback"] {
        let mut package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{}} struct Caller {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            impl Retire for Caller {{fn drop(*self) {{}}}}
            typealias Callback=%{{ticket:Ticket}} fn()->Ticket;
            effect poll()->bool;fn subject()->#poll bool {{poll()#}}
            fn build()->{result} where ensures true {{let caller=Caller {{}};subject()# {{poll()=>do {{let ticket=Ticket {{}};return || ticket;}},}};let ticket=Ticket {{}};|| ticket}}
            fn entry() {{let callback=build();}}"));
        let build=package.functions.iter_mut().find(|f|f.name=="build").unwrap();
        build.postconditions[0].kind=nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","build","subject","drop Ticket","drop Caller"]);
    }
}

#[test]
fn scheduled_handler_returned_callable_cannot_consume_capture_twice() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        typealias Callback=%{ticket:Ticket} fn()->Ticket;
        effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn build()->Callback {subject()# {poll()=>do {let ticket=Ticket {};return || ticket;},};let ticket=Ticket {};|| ticket}
        fn entry() {let callback=build();let first=callback();let second=callback();}");
    assert!(source_executable(&package,"entry").is_err(),"lexical return must not duplicate a closure's owned environment");
}

#[test]
fn scheduled_handler_returned_callable_retains_nested_callable_captures() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Callback=%{helper:fn(bool)->bool} fn(bool)->bool;
        fn identity(value:bool)->bool {value} fn negate(value:bool)->bool {not value}
        effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn build(flag:bool)->Callback {subject()# {poll()=>do {
            let helper=if flag {identity} else {negate};return |value:bool| helper(value);
        },};let helper=identity;|value:bool| helper(value)}
        fn entry(flag:bool)->bool {let callback=build(flag);callback(true)==flag}");
    let report=cleanup_report(&package,"build").unwrap();
    let dump=report.to_string();
    assert!(dump.contains("(handler-return (source"),"{dump}");
    assert!(dump.contains("(callable-evidence"),"{dump}");
    let executable=source_executable(&package,"entry").unwrap();
    for flag in [false,true] {
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![Value::Bool(flag)]).unwrap(),Value::Bool(true));
        let names=executed_names(&package,&executable,vm.events());
        assert_eq!(names.last().unwrap(),if flag {"identity"} else {"negate"});
    }
}

#[test]
fn scheduled_handler_returned_callable_does_not_restore_consumed_capture_before_return() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        typealias Callback=%{ticket:Ticket} fn()->Ticket;
        effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn build()->Callback {subject()# {poll()=>do {let ticket=Ticket {};let callback=|| ticket;let consumed=callback();return callback;},};let ticket=Ticket {};|| ticket}
        fn entry() {let callback=build();let ticket=callback();}");
    assert!(source_executable(&package,"entry").is_err(),"returning code evidence must not restore a missing capture");
}

#[test]
fn scheduled_handler_returned_callable_cannot_hide_handler_local_pointer() {
    for (types, body) in [
        ("typealias Callback=%{pointer:*bool} fn()->*bool;", "let local=true;let pointer=local.ref;return || pointer;"),
        ("typealias Callback=%{pointer:*bool} fn()->*bool;", "let local=true;let pointer=local.ref;let callback=|| pointer;return callback;"),
        ("struct Holder {pointer:*bool} typealias Callback=%{holder:Holder} fn()->Holder;", "let local=true;let holder=Holder {pointer:local.ref};return || holder;"),
        ("typealias Inner=%{pointer:*bool} fn()->*bool;typealias Callback=%{inner:Inner} fn()->Inner;", "let local=true;let pointer=local.ref;let inner=|| pointer;return || inner;"),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {types}
            effect poll()->Callback;fn subject()->#poll Callback {{poll()#}}
            fn build()->Callback {{subject()# {{poll()=>do {{{body}}},}}}}"));
        // Reject at the producing function, even without invoking the callback.
        let error=source_executable(&package,"build").unwrap_err();
        assert!(error.contains("escaping provenance"),"{body}: {error}");
    }
}

#[test]
fn scheduled_borrowed_capture_reads_writes_and_returns_the_actual_pointer() {
    use comptime_interpretor::{Interpreter,Value};
    for source in [
        "fn entry()->bool {var flag=false;let pointer=flag.ref;let update=|| {pointer.* = true;};update();flag}",
        "fn entry()->bool {let flag=true;let pointer=flag.ref;let read=|| pointer.*;read()}",
        "fn entry()->bool {var flag=false;let pointer=flag.ref;let address=|| pointer;let result=address();result.* = true;flag}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {source}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{source}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{source}");
    }
}

#[test]
fn scheduled_borrowed_capture_combines_arguments_aliases_and_pointer_snapshot() {
    use comptime_interpretor::{Interpreter,Value};
    for source in [
        "fn entry()->bool {var first=false;let second=true;let pointer=first.ref;let update=|argument:*bool| {pointer.* = argument.*;};update(second.ref);first}",
        "fn entry()->bool {var flag=false;let first=flag.ref;let second=flag.ref;let update=|| {first.* = true;second.*};update()}",
        "fn entry()->bool {let first=true;let second=false;var pointer=first.ref;let read=|| pointer.*;pointer=second.ref;read()}",
        "fn entry()->bool {var flag=false;let pointer=flag.ref;let toggle=|| {pointer.* = not pointer.*;pointer.*};let first=toggle();let second=toggle();first and not second and not flag}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {source}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{source}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{source}");
    }
}

#[test]
fn scheduled_borrowed_capture_coexists_with_owned_capture_and_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    for body in [
        "let callback=|| {let result=ticket;pointer.* = true;result};let result=callback();flag",
        "let callback=|| {pointer.* = true;let result=ticket;result};let result=callback();flag",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            fn entry()->bool {{var flag=false;let pointer=flag.ref;let ticket=Ticket {{}};{body}}}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{body}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Ticket"]);
    }
}

#[test]
fn scheduled_borrowed_capture_failure_cleans_owned_environment_and_caller() {
    use comptime_interpretor::{Interpreter,ErrorKind};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {} struct Caller {} impl Retire for Ticket {fn drop(*self) {}}
        impl Retire for Caller {fn drop(*self) {}}
        fn entry() {let caller=Caller {};var flag=false;let pointer=flag.ref;let ticket=Ticket {};
            let callback=|| {pointer.* = true;assumes false;let moved=ticket;};callback();}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap_err().kind,ErrorKind::ProgramContractRejected);
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Ticket","drop Caller"]);
}

#[test]
fn scheduled_borrowed_capture_rejects_moved_targets_rebinding_and_escaping_results() {
    for source in [
        "struct Held {flag:bool} impl Retire for Held {fn drop(*self) {}} fn entry()->bool {let held=Held {flag:true};let pointer=held.ref;let read=|| pointer.flag.*;let moved=held;read()}",
        "fn entry()->*bool {let flag=true;let pointer=flag.ref;let address=|| pointer;address()}",
        "fn entry()->bool {let first=true;let second=false;var pointer=first.ref;let other=second.ref;let replace=|| {pointer=other;pointer.*};replace()}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {source}"));
        assert!(source_executable(&package,"entry").is_err(),"{source}");
    }
}

#[test]
fn scheduled_borrowed_capture_compositions_preserve_storage_origins() {
    use comptime_interpretor::{Interpreter,Value};
    for source in [
        "struct Actions {callback:Callback} fn entry()->bool {let flag=true;let pointer=flag.ref;let actions=Actions {callback:|| pointer.*};actions.callback()}",
        "fn entry()->bool {let flag=true;let pointer=flag.ref;let (callback,other)=(|| pointer.*,false);callback() and not other}",
        "enum Action {some(Callback),none} fn entry()->bool {let flag=true;let pointer=flag.ref;let action=Action.some(|| pointer.*);action match {.some(callback)=>callback(),.none=>false}}",
        "fn entry()->bool {let first=true;let second=false;let left=first.ref;let right=second.ref;let choose=true;let callback:Callback=if choose {|| left.*} else {|| not right.*};callback()}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} typealias Callback=%{{pointer:*bool}} fn()->bool;{source}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{source}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{source}");
    }
}

#[test]
fn scheduled_borrowed_capture_follows_indirect_and_field_targets() {
    use comptime_interpretor::{Interpreter,Value};
    for source in [
        "fn entry()->bool {let flag=true;let pointer=flag.ref;let indirect=pointer.ref;let read=|| indirect.*.*;read()}",
        "struct Held {flag:bool} impl Retire for Held {fn drop(*self) {}} fn entry()->bool {let held=Held {flag:true};let pointer=held.ref;let read=|| pointer.flag.*;read()}",
        "fn entry()->bool {var flag=false;let pointer=flag.ref;let read=|ignored:bool| pointer.*;read(do {flag=true;true})}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {source}"));
        let executable=source_executable(&package,"entry").unwrap_or_else(|e|panic!("{source}: {e}"));
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{source}");
    }
}

#[test]
fn divergent_loop_return_cleans_iteration_before_function_scope() {
    use comptime_interpretor::{Interpreter, Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Iteration {}
        impl Retire for Caller {fn drop(*self) {}}
        impl Retire for Iteration {fn drop(*self) {}}
        fn entry()->bool {
            let caller=Caller {};
            while true {let iteration=Iteration {};return true;}
        }");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Iteration","drop Caller"]);
}

#[test]
fn definite_control_exit_preserves_cleanup_and_result_delivery() {
    use comptime_interpretor::{Interpreter, Value};
    for body in [
        "if true {let branch=Branch {};return true;}",
        "true and do {let branch=Branch {};return true;};",
        "false or do {let branch=Branch {};return true;};",
        "when {false=>return false,true=>do {let branch=Branch {};return true;}}",
        "when {true=>do {let branch=Branch {};return true;},else=>false}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Caller {{}} struct Branch {{}}
            impl Retire for Caller {{fn drop(*self) {{}}}}
            impl Retire for Branch {{fn drop(*self) {{}}}}
            fn entry()->bool {{let caller=Caller {{}};{body}}}"));
        let executable=source_executable(&package,"entry").unwrap();
        let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
            .with_cleanup_schedule(&executable.schedule).unwrap();
        assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true),"{body}");
        assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Branch","drop Caller"],"{body}");
    }
}

#[test]
fn inline_function_retains_owned_argument_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Input {} struct Local {}
        impl Retire for Caller {fn drop(*self) {}}
        impl Retire for Input {fn drop(*self) {}}
        impl Retire for Local {fn drop(*self) {}}
        inline fn consume(input:Input)->bool {let local=Local {};true}
        fn entry()->bool {let caller=Caller {};consume(Input {})}");
    let executable=source_executable(&package,"entry").unwrap();
    assert!(executable.graph.functions.iter().any(|f|f.function.name=="consume" && f.function.inline));
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","consume","drop Local","drop Input","drop Caller"]);
}

#[test]
fn compound_assignment_prepared_pointer_obeys_resource_scope() {
    use comptime_interpretor::{Interpreter,Value};
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Byte=.u8'builtin;struct Counter {value:Byte}
        impl Retire for Counter {fn drop(*self) {}}
        inline fn increment(pointer:*Byte) {assumes pointer.* < 100;pointer.* += 1;}
        fn entry()->bool {let counter=Counter {value:4};increment(counter.value.ref);counter.value == 5}");
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","increment","drop Counter"]);
}

#[test]
fn optional_inline_pass_preserves_owned_parameters_and_handler_escape_cleanup() {
    use comptime_interpretor::{Interpreter,Value};
    let mut package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Caller {} struct Input {} struct Body {} struct Handler {}
        impl Retire for Caller {fn drop(*self) {}} impl Retire for Input {fn drop(*self) {}}
        impl Retire for Body {fn drop(*self) {}} impl Retire for Handler {fn drop(*self) {}}
        inline effect poll()->bool;
        inline fn subject(input:Input)->#poll bool {let body=Body {};poll()#}
        fn entry()->bool {let caller=Caller {};subject(Input {})# {
            poll()=>do {let handler=Handler {};return true;},
        }}");
    let report=elaboration::program::inlining::expand(&mut package,&mut 100_000).unwrap();
    assert_eq!(report.expanded,1);
    let executable=source_executable(&package,"entry").unwrap();
    let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
        .with_cleanup_schedule(&executable.schedule).unwrap();
    assert_eq!(vm.invoke_instance(&executable.entry,vec![]).unwrap(),Value::Bool(true));
    assert_eq!(executed_names(&package,&executable,vm.events()),["entry","drop Handler","drop Body","drop Input","drop Caller"]);
}

#[test]
fn optional_inline_contract_checks_match_call_cleanup_on_every_exit() {
    use comptime_interpretor::{ErrorKind,Interpreter,Value};
    for early in [false,true] {
        for failure in ["none","entry","result"] {
            let original=compile("trait Duplicate {} trait Retire {fn drop(*self);}
                struct Caller {} struct Input {} struct Body {} struct Output {}
                impl Retire for Caller {fn drop(*self) {}} impl Retire for Input {fn drop(*self) {}}
                impl Retire for Body {fn drop(*self) {}} impl Retire for Output {fn drop(*self) {}}
                inline fn produce(input:Input,early:bool)->Output where requires early == early, ensures true {
                    let body=Body {};if early {return Output {};}Output {}
                }
                fn entry(early:bool)->bool {let caller=Caller {};let output=produce(Input {},early);true}");
            let mut expected=None;
            for expanded in [false,true] {
                let mut package=original.clone();
                let function=package.functions.iter_mut().find(|f|f.name=="produce").unwrap();
                let predicate=match failure {
                    "entry"=>function.preconditions.first_mut(),
                    "result"=>function.postconditions.first_mut(),
                    _=>None,
                };
                assert_eq!(predicate.is_some(),failure!="none");
                if let Some(predicate)=predicate {
                    predicate.kind=nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
                }
                if expanded {
                    let report=elaboration::program::inlining::expand(&mut package,&mut 100_000).unwrap();
                    assert_eq!(report.expanded,1);
                }
                let executable=source_executable(&package,"entry").unwrap();
                let mut vm=Interpreter::from_iir(&executable.graph,Default::default()).unwrap()
                    .with_cleanup_schedule(&executable.schedule).unwrap();
                let result=vm.invoke_instance(&executable.entry,vec![Value::Bool(early)]).map_err(|error|error.kind);
                assert_eq!(result,if failure=="none" {Ok(Value::Bool(true))} else {Err(ErrorKind::ProgramContractRejected)},"{failure}/{early}/{expanded}");
                let names=executed_names(&package,&executable,vm.events()).into_iter().filter(|name|name!="produce").collect::<Vec<_>>();
                if let Some(expected)=&expected {assert_eq!(&names,expected,"{failure}/{early}");}
                else {expected=Some(names);}
            }
        }
    }
}
