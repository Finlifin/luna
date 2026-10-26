//! Pointer origins across effect computation and installation boundaries.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn optional_effect_transfers_preserve_presence_and_pointer_lifetimes() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        typealias Maybe=?*bool;
        effect relay(value:Maybe)->Maybe;
        fn forward(value:Maybe)->#relay Maybe {relay(value)#}";
    let package = support::compile(&format!(
        "{prefix}
        fn run()->bool {{
            var flag=false;
            let present:Maybe=flag.ref;
            let delivered=forward(present)# {{relay(value)=>value,}};
            delivered match {{pointer?=>{{pointer.* = true;}},null=>{{}},}};
            let absent:Maybe=null;
            let empty=forward(absent)# {{relay(value)=>value,}};
            flag and (empty match {{_?=>false,null=>true}})
        }}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let dangling = support::compile(&format!(
        "{prefix}
        fn run()->Maybe {{var local=false;
            forward(local.ref)# {{relay(value)=>value,}}
        }}"
    ));
    let error = luna::execution::interpret_with_protocols(
        &dangling,
        "run",
        "Duplicate",
        "Retire",
        1_000_000,
    )
    .unwrap_err();
    assert!(error.contains("move-error"), "{error}");
}

#[test]
fn conditional_memory_joins_same_operation_contracts_without_order_dependence() {
    use elaboration::verification::memory;
    use std::collections::BTreeSet;
    let package = support::compile(
        "effect choose(left:*bool,right:*bool)->*bool;
        fn subject(left:*bool,right:*bool)->#choose *bool {choose(left,right)#}",
    );
    let operation = &package.effects[0];
    let premise = |selected, discarded| memory::EffectInput {
        parameter_accesses: Default::default(),
        operation: nir::effects::Operation {
            declaration: operation.declaration,
            arguments: vec![],
        },
        abi: operation.abi,
        signature: operation.signature.clone(),
        forwarded_parameters: Some(BTreeSet::from([selected])),
        owned_inputs: false,
        copy_parameters: BTreeSet::from([discarded]),
    };
    let infer = |inputs: &[memory::EffectInput]| {
        memory::infer_with_effect_inputs(
            &package,
            support::id(&package, "subject"),
            &[],
            &Default::default(),
            &Default::default(),
            inputs,
            &mut 200_000,
        )
    };
    for inputs in [
        [premise(0, 1), premise(1, 0)],
        [premise(1, 0), premise(0, 1)],
    ] {
        let summary = infer(&inputs).unwrap();
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .map(|target| target.parameter)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([0, 1])
        );
        assert!(
            summary
                .accesses
                .iter()
                .all(|access| !access.read_value && !access.write)
        );
        assert!(summary.to_string().contains("(copy-inputs"));
    }
    let mut missing = premise(0, 1);
    missing.copy_parameters.clear();
    assert!(infer(&[missing]).is_err());
    assert!(infer(&[premise(0, 2)]).is_err());
    assert!(infer(&[premise(0, 1)]).is_ok());
}

#[test]
fn scalar_effect_completion_can_discard_a_checked_copy_pointer() {
    for handler in [
        "keep(flag,_)=>flag,",
        "keep(true as flag,_)=>flag,keep(false as flag,_)=>flag,",
        "keep((true or false) as flag,_)=>flag,",
        "keep((true as flag) or (false as flag),_)=>flag,",
        "keep(flag,_)=>do {if flag {resume flag;};flag},",
        "keep(flag,_)=>flag match {true as selected=>selected,false as selected=>selected},",
    ] {
        for flag in ["true", "false"] {
            let source = format!(
                "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
                effect keep(flag:bool,pointer:*bool)->bool;
                fn subject()->#keep bool {{var local=false;keep({flag},local.ref)#}}
                fn run()->bool {{subject()# {{{handler}}}}}"
            );
            let package = support::compile(&source);
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    200_000
                )
                .unwrap_or_else(|error| panic!("{handler} {flag}: {error}")),
                Value::Bool(flag == "true")
            );
        }
    }
}

#[test]
fn effect_discard_copy_premise_cannot_hide_a_custom_destructor() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Link {target:*bool} impl Duplicate for Link {}
        impl Retire for Link {fn drop(*self) {self.target.*.* = true;}}
        effect choose(left:Link,right:Link)->Link;
        fn run()->bool {var first=false;var second=false;
            let value=choose(Link{target:first.ref},Link{target:second.ref})# {choose(left,right)=>left,};
            value.target.*
        }";
    let package = support::compile(source);
    let failure =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap_err();
    assert!(
        failure.starts_with("cannot prepare resource-checked execution"),
        "{failure}"
    );
}

#[test]
fn ordinary_effect_can_ignore_checked_copy_pointer_inputs() {
    for (declarations, ty, first, second, access) in [
        ("", "*bool", "first.ref", "second.ref", "value.*"),
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{target:first.ref}",
            "Link{target:second.ref}",
            "value.target.*",
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} typealias Payload={ty};
            effect choose(left:Payload,right:Payload)->Payload;
            fn forward(left:Payload,right:Payload)->#choose Payload {{choose(left,right)#}}"
        );
        for (body, expected) in [
            ("left", "first and not second"),
            ("right", "not first and second"),
        ] {
            let source = format!(
                "{prefix} fn run()->bool {{var first=false;var second=false;
                let value=forward({first},{second})# {{choose(left,right)=>{body},}};
                {access} = true;{expected}
            }}"
            );
            let package = support::compile(&source);
            let op = &package.effects[0];
            let premise =
                elaboration::verification::memory::EffectInput {
                    parameter_accesses: Default::default(),
                    operation: nir::effects::Operation {
                        declaration: op.declaration,
                        arguments: vec![],
                    },
                    abi: op.abi,
                    signature: op.signature.clone(),
                    forwarded_parameters: Some(std::collections::BTreeSet::from([
                        if body == "left" { 0 } else { 1 },
                    ])),
                    owned_inputs: false,
                    copy_parameters: std::collections::BTreeSet::from([0, 1]),
                };
            let summary = elaboration::verification::memory::infer_with_effect_inputs(
                &package,
                support::id(&package, "forward"),
                &[],
                &Default::default(),
                &Default::default(),
                &[premise],
                &mut 200_000,
            )
            .unwrap();
            let targets = if ty == "*bool" {
                summary.result_targets.as_ref().unwrap()
            } else {
                summary
                    .pointer_fields
                    .get(&vec![0])
                    .unwrap()
                    .as_ref()
                    .unwrap()
            };
            assert_eq!(
                targets
                    .iter()
                    .map(|target| target.parameter)
                    .collect::<std::collections::BTreeSet<_>>(),
                std::collections::BTreeSet::from([if body == "left" { 0 } else { 1 }])
            );
            elaboration::verification::memory::infer_in_package(
                &package,
                support::id(&package, "run"),
                &[],
                &mut 200_000,
            )
            .unwrap();
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    200_000,
                )
                .unwrap_or_else(|reason| panic!("{ty} {body}: {reason}")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn nested_effect_installations_keep_distinct_pointer_origins() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        effect choose(left:*bool,right:*bool)->*bool;
        fn inner(left:*bool,right:*bool)->#choose *bool {
            choose(left,right)# {choose(left,right)=>right,}
        }
        fn run()->bool {var first=false;var second=false;
            let p=inner(first.ref,second.ref)# {choose(left,right)=>left,};
            p.* = true;not first and second
        }";
    let package = support::compile(source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 200_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000,)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn conditional_effect_memory_keeps_all_possible_parameter_origins() {
    use elaboration::verification::memory;
    use std::collections::BTreeSet;
    let package = support::compile(
        "effect choose(flag:bool,left:*bool,right:*bool)->*bool;
        fn subject(flag:bool,left:*bool,right:*bool,third:*bool)->#choose *bool {
            var selected=left;choose(flag,selected,do {selected=third;right})#
        }",
    );
    let operation = &package.effects[0];
    let input = memory::EffectInput {
        parameter_accesses: Default::default(),
        owned_inputs: false,
        copy_parameters: Default::default(),
        operation: nir::effects::Operation {
            declaration: operation.declaration,
            arguments: vec![],
        },
        abi: operation.abi,
        signature: operation.signature.clone(),
        forwarded_parameters: Some(BTreeSet::from([1, 2])),
    };
    let infer = |input: memory::EffectInput, fuel: &mut usize| {
        memory::infer_with_effect_inputs(
            &package,
            support::id(&package, "subject"),
            &[],
            &Default::default(),
            &Default::default(),
            &[input],
            fuel,
        )
    };
    let summary = infer(input.clone(), &mut 100_000).unwrap();
    assert_eq!(
        summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|target| target.parameter)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2])
    );
    assert!(
        summary
            .accesses
            .iter()
            .all(|access| !access.read_value && !access.write)
    );
    assert!(summary.to_string().contains("(forwarded-parameters 1 2)"));
    for invalid in [
        BTreeSet::new(),
        BTreeSet::from([0, 1, 2]),
        BTreeSet::from([1, 3]),
    ] {
        let mut input = input.clone();
        input.forwarded_parameters = Some(invalid);
        assert!(infer(input, &mut 100_000).is_err());
    }
    assert!(infer(input.clone(), &mut 0).is_err());
    assert_eq!(summary, infer(input, &mut 100_000).unwrap());
}

#[test]
fn ordinary_effect_joins_variant_qualified_pointer_sources() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        enum Choice {none,left(*bool),right(*bool)} impl Duplicate for Choice {}
        effect choose(flag:bool,left:Choice,right:Choice)->Choice;
        fn install(flag:bool,left:Choice,right:Choice)->Choice {
            choose(flag,left,right)# {choose(flag,left,right)=>if flag {left} else {right},}
        }
        fn exercise(flag:bool)->bool {var first=false;var second=false;
            let value=install(flag,Choice.left(first.ref),Choice.right(second.ref));
            value match {.none=>false,.left(p)=>{p.* = true;first and not second},.right(p)=>{p.* = true;not first and second}}
        }
        fn run()->bool {var flag=false;
            exercise(true) and exercise(false) and
            (install(true,Choice.none,Choice.left(flag.ref)) match {.none=>true,.left(p)=>p.*,.right(p)=>p.*})
        }";
    let package = support::compile(source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 400_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 400_000,)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_effect_joins_borrowed_callback_candidates() {
    for (flag, expected) in [
        ("true", "first and not second"),
        ("false", "not first and second"),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            effect choose(flag:bool,left:%c fn()->bool,right:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
            fn forward(flag:bool,left:%c fn()->bool,right:%c fn()->bool)->#choose<c> %c fn()->bool where c:Type.vars.Closure {{choose(flag,left,right)#}}
            fn install(flag:bool,left:%c fn()->bool,right:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {{
                forward(flag,left,right)# {{choose<c>(flag,left,right)=>if flag {{left}} else {{right}},}}
            }}
            fn run()->bool {{var first=false;var second=false;let p=first.ref;let q=second.ref;
                let callback=install({flag},|| do {{p.* = true;p.*}},|| do {{q.* = true;q.*}});
                callback() and {expected}
            }}");
        let package = support::compile(&source);
        elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "run"),
            &[],
            &mut 400_000,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                400_000,
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn ordinary_effect_joins_owned_callback_candidates_without_duplicating_owners() {
    for body in [
        "if flag {left} else {right}",
        "(do {let selected=if flag {left} else {right};selected}) match {chosen=>chosen}",
        "(do {if flag {resume left;};right}) match {chosen=>chosen}",
        "do {if flag {resume left;};right}",
        "do {flag match {selected if selected=>do {resume left;},_=>()};resume right;}",
        "do {let chosen=if flag {resume left;} else {right};chosen}",
    ] {
        for flag in ["true", "false"] {
            for calls in [1, 2] {
                let source=format!("typealias Type=.Type'builtin;
                trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                impl Duplicate for bool {{}}
                struct Ticket {{flag:bool}} impl Retire for Ticket {{fn drop(*self) {{}}}}
                fn take(ticket:Ticket)->bool {{ticket.flag}}
                effect choose(flag:bool,left:%c fn()->bool,right:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
                fn install(flag:bool,left:%c fn()->bool,right:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {{
                    choose(flag,left,right)# {{choose<c>(flag,left,right)=>{body},}}
                }}
                fn run()->bool {{let first=Ticket{{flag:true}};let second=Ticket{{flag:false}};
                    let callback=install({flag},|| take(first),|| take(second));
                    {call}
                }}",call=if calls==1 {"callback()"} else {"let first=callback();callback()"});
                let package = support::compile(&source);
                let result = luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    300_000,
                );
                if calls == 1 {
                    assert_eq!(result.unwrap(), Value::Bool(flag == "true"));
                } else {
                    let failure = result.unwrap_err();
                    assert!(
                        failure.starts_with("cannot prepare resource-checked execution")
                            && failure.contains("unavailable"),
                        "{failure}"
                    );
                }
            }
        }
    }
}

#[test]
fn ordinary_effect_joins_alternative_pointer_sources() {
    for handler in [
        "choose(flag,left,right)# {choose(flag,left,right)=>if flag {left} else {right},}",
        "choose(flag,left,right)# {choose(flag,left,right)=>(do {let selected=if flag {left} else {right};selected}) match {chosen=>chosen},}",
        "choose(flag,left,right)# {choose(flag,left,right)=>(do {if flag {resume left;};right}) match {chosen=>chosen},}",
        "choose(flag,left,right)# {choose(flag,left,right)=>do {let true=(flag) else {resume right;}; left},}",
        "choose(flag,left,right)# {choose(flag,left,right)=>do {let (selected if selected)=(flag) else {resume right;}; left},}",
        "choose(flag,left,right)# {choose(flag,left,right)=>do {if flag {resume left;};right},}",
        "choose(flag,left,right)# {choose(flag,left,right)=>do {if flag {resume left;} else {resume right;};},}",
        "choose(flag,left,right)# {choose(true,left,right)=>left,choose(false,left,right)=>right,}",
        "(choose(flag,left,right)# {choose(flag,left,right) if flag=>left,})# {choose(flag,left,right)=>if flag {left} else {right},}",
    ] {
        let declarations = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            effect choose(flag:bool,left:*bool,right:*bool)->*bool;
            fn install(flag:bool,left:*bool,right:*bool)->*bool {{{handler}}}"
        );
        for (flag, expected) in [
            ("true", "first and not second"),
            ("false", "not first and second"),
        ] {
            let source = format!(
                "{declarations} fn run()->bool {{var first=false;var second=false;
                let p=install({flag},first.ref,second.ref);p.* = true;{expected}}}"
            );
            let package = support::compile_with_protocols(&source, &Default::default())
                .unwrap_or_else(|error| panic!("{handler}: {error}"));
            elaboration::verification::memory::infer_in_package(
                &package,
                support::id(&package, "run"),
                &[],
                &mut 300_000,
            )
            .unwrap_or_else(|reason| panic!("{handler}: {reason}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    300_000,
                )
                .unwrap_or_else(|reason| panic!("{handler}: {reason}")),
                Value::Bool(true)
            );
        }
        let source = format!(
            "{declarations} fn run()->bool {{var live=false;
            let p=do {{var temporary=false;install(false,live.ref,temporary.ref)}};p.*}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{handler}: {error}"));
        let failure = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            300_000,
        )
        .unwrap_err();
        assert!(
            failure.starts_with("cannot prepare resource-checked execution"),
            "{failure}"
        );
    }
}

#[test]
fn ordinary_effect_transfer_tracks_conditional_bodies_and_guards() {
    for handler in [
        "relay(flag,value)# {relay(flag,value)=>if flag {value} else {value},}",
        "relay(flag,value)# {relay(flag,value)=>do {let true=(flag) else {resume value;}; value},}",
        "relay(flag,value)# {relay(flag,value)=>do {let (selected if selected)=(flag) else {resume value;}; value},}",
        "relay(flag,value)# {relay(flag,value)=>do {if flag {resume value;} else {resume value;};},}",
        "relay(flag,value)# {relay(flag,value)=>do {if flag {resume value;};value},}",
        "relay(flag,value)# {relay(flag,value)=>do {flag match {true=>do {resume value;},false=>do {resume value;}};},}",
        "relay(flag,value)# {relay(flag,value)=>do {flag match {true=>do {resume value;},false=>()};resume value;},}",
        "relay(flag,value)# {relay(flag,value)=>do {let choose=not flag;if choose {let saved=value;saved} else {value}},}",
        "relay(flag,value)# {relay(flag,value)=>flag match {true=>value,false=>do {resume value;}},}",
        "relay(flag,value)# {relay(flag,value)=>flag match {selected=>if selected {value} else {value}},}",
        "relay(flag,value)# {relay(flag,value)=>flag match {selected if selected=>value,other=>if other {value} else {value}},}",
        "relay(flag,value)# {relay(flag,value)=>(not flag) match {selected=>selected match {again=>if again {value} else {value}}},}",
        "relay(flag,value)# {relay(flag,value) if flag=>value,relay(flag,value)=>value,}",
        "relay(flag,value)# {relay(flag,value) if do {let choice=not flag;not choice}=>value,relay(flag,value)=>value,}",
        "relay(flag,value)# {relay(flag,value) if (if flag {true} else {false})=>value,relay(flag,value)=>value,}",
        "relay(flag,value)# {relay(flag,value) if (flag match {true=>true,false=>false})=>value,relay(flag,value)=>value,}",
        "relay(flag,value)# {relay(flag,value)=>if do {let selected=flag match {choice=>choice};if selected {true} else {false}} {value} else {value},}",
        "(relay(flag,value)# {relay(flag,value) if flag=>value,})# {relay(flag,value)=>value,}",
    ] {
        let declarations = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            effect relay(flag:bool,value:*bool)->*bool;
            fn install(flag:bool,value:*bool)->*bool {{{handler}}}"
        );
        for flag in ["true", "false"] {
            let source = format!(
                "{declarations} fn run()->bool {{var flag=false;
                let p=install({flag},flag.ref);p.* = true;flag}}"
            );
            let package = support::compile_with_protocols(&source, &Default::default())
                .unwrap_or_else(|error| panic!("{handler}: {error}"));
            elaboration::verification::memory::infer_in_package(
                &package,
                support::id(&package, "run"),
                &[],
                &mut 200_000,
            )
            .unwrap_or_else(|reason| panic!("{handler}: {reason}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    200_000,
                )
                .unwrap_or_else(|reason| panic!("{handler}: {reason}")),
                Value::Bool(true)
            );
            let invalid = format!(
                "{declarations} fn run()->bool {{let p=do {{var flag=false;install({flag},flag.ref)}};p.*}}"
            );
            let package = support::compile(&invalid);
            let failure = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000,
            )
            .unwrap_err();
            assert!(
                failure.starts_with("cannot prepare resource-checked execution"),
                "{failure}"
            );
        }
    }
}

#[test]
fn ordinary_effect_preserves_tuple_newtype_and_variant_pointer_origins() {
    for (declarations, ty, construct, access) in [
        (
            "",
            "Tuple<*bool,*bool>",
            "(flag.ref,flag.ref)",
            "value match {(p,_)=>p.*}",
        ),
        (
            "newtype Pointer=*bool;impl Duplicate for Pointer {}",
            "Pointer",
            "flag.ref.as(Pointer)",
            "value.as(*bool).*",
        ),
        (
            "enum Choice {none,some(*bool)} impl Duplicate for Choice {}",
            "Choice",
            "Choice.some(flag.ref)",
            "value match {.none=>false,.some(p)=>p.*}",
        ),
    ] {
        let declarations = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} typealias Payload={ty};
            effect relay(value:Payload)->Payload;
            fn forward(value:Payload)->#relay Payload {{relay(value)#}}
            fn install(value:Payload)->Payload {{forward(value)# {{relay(value)=>value,}}}}"
        );
        let valid = format!(
            "{declarations} fn run()->bool {{var flag=true;let value=install({construct});{access}}}"
        );
        let package = support::compile_with_tuple(&valid);
        elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "run"),
            &[],
            &mut 200_000,
        )
        .unwrap_or_else(|reason| panic!("{ty}: {reason}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000,
            )
            .unwrap_or_else(|reason| panic!("{ty}: {reason}")),
            Value::Bool(true)
        );
        let scalar = format!(
            "{declarations}
            fn subject()->#relay bool {{var flag=true;let value=relay({construct})#;{access}}}
            fn run()->bool {{subject()# {{relay(value)=>value,}}}}"
        );
        let package = support::compile_with_tuple(&scalar);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000,
            )
            .unwrap_or_else(|reason| panic!("scalar result {ty}: {reason}")),
            Value::Bool(true)
        );
        let invalid = format!(
            "{declarations} fn run()->bool {{let value=do {{var flag=true;install({construct})}};{access}}}"
        );
        let package = support::compile_with_tuple(&invalid);
        let failure = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            failure.starts_with("cannot prepare resource-checked execution"),
            "{ty}: {failure}"
        );
    }
}

#[test]
fn ordinary_effect_aggregate_delivery_preserves_capture_consumption() {
    for (calls, accepted) in [
        ("bundle.callback()", true),
        ("bundle.callback() and bundle.callback()", false),
    ] {
        let source=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            fn consume(ticket:Ticket)->bool {{true}}
            struct Bundle where C {{callback:C,target:*bool}}
            fn pack(callback:C,target:*bool)->Bundle<C> where C {{Bundle<C>{{callback:callback,target:target}}}}
            effect relay(value:Bundle<C>)->Bundle<C> where C;
            fn install(value:Bundle<C>)->Bundle<C> where C {{relay(value)# {{relay<C>(value)=>value,}}}}
            fn run()->bool {{var flag=false;let ticket=Ticket{{}};
                let bundle=install(pack(|| consume(ticket),flag.ref));
                bundle.target.* = true;{calls} and flag
            }}");
        let package = support::compile(&source);
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        );
        if accepted {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            let failure = result.unwrap_err();
            assert!(
                failure.starts_with("cannot prepare resource-checked execution")
                    && failure.contains("unavailable"),
                "{failure}"
            );
        }
    }
}

#[test]
fn ordinary_effect_preserves_variant_guards_on_pointer_fields() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        enum Choice {none,left(*bool),right(*bool)} impl Duplicate for Choice {}
        effect relay(value:Choice)->Choice;
        fn install(value:Choice)->Choice {relay(value)# {relay(value)=>value,}}
        fn exercise(select:bool)->bool {var first=false;var second=true;
            let input=if select {Choice.left(first.ref)} else {Choice.right(second.ref)};
            let value=install(input);
            value match {.none=>false,.left(p)=>{p.* = true;first and second},.right(p)=>{p.* = false;not first and not second}}
        }
        fn run()->bool {exercise(true) and exercise(false) and
            (install(Choice.none) match {.none=>true,.left(p)=>p.*,.right(p)=>p.*})
        }";
    let package = support::compile(source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 300_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300_000,)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_effect_preserves_code_and_pointer_fields_together() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Bundle where C {callback:C,target:*bool}
        fn pack(callback:C,target:*bool)->Bundle<C> where C {Bundle<C>{callback:callback,target:target}}
        effect relay(value:Bundle<C>)->Bundle<C> where C;
        fn forward(value:Bundle<C>)->#relay<C> Bundle<C> where C {relay(value)#}
        fn install(value:Bundle<C>)->Bundle<C> where C {forward(value)# {relay<C>(value)=>value,}}
        fn run()->bool {var first=false;var second=true;let p=first.ref;
            let bundle=install(pack(|| do {p.* = true;p.*},second.ref));
            bundle.target.* = false;bundle.callback() and first and not second
        }";
    let package = support::compile(source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 200_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000,)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_effect_forwards_nested_pointer_records() {
    let declarations = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Link {target:*bool} impl Duplicate for Link {}
        struct Pair {first:Link,second:Link} impl Duplicate for Pair {}
        effect relay(value:Pair)->Pair;
        fn forward(value:Pair)->#relay Pair {relay(value)#}
        fn install(value:Pair)->Pair {forward(value)# {relay(value)=>value,}}";
    let source = format!("{declarations}
        fn run()->bool {{var first=false;var second=true;
            let pair=install(Pair{{first:Link{{target:first.ref}},second:Link{{target:second.ref}}}});
            pair.first.target.* = true;pair.second.target.* = false;
            first and not second
        }}");
    let package = support::compile(&source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 200_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000,)
            .unwrap(),
        Value::Bool(true)
    );
    let invalid = format!(
        "{declarations}
        fn run()->bool {{let pair=do {{var flag=false;
            install(Pair{{first:Link{{target:flag.ref}},second:Link{{target:flag.ref}}}})
        }};pair.first.target.*}}"
    );
    let package = support::compile(&invalid);
    let failure =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap_err();
    assert!(
        failure.starts_with("cannot prepare resource-checked execution"),
        "{failure}"
    );
}

#[test]
fn ordinary_handler_forwards_pointer_identity() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        effect relay(p:*bool)->*bool;
        fn run()->bool {var flag=false;
            let p=relay(flag.ref)# {relay(p)=>p,};
            p.* = true;flag
        }";
    let package = support::compile(source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pointer_delivering_effect_requires_context_even_for_scalar_callee_results() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        effect relay(p:*bool)->*bool;
        fn subject()->#relay bool {var flag=false;let p=relay(flag.ref)#;p.* = true;flag}
        fn run()->bool {subject()# {relay(p)=>p,}}";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_pointer_delivery_preserves_calls_aliases_and_resume() {
    for body in ["p", "do {let retained=p;retained}", "do {resume p;}"] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            effect relay(p:*bool)->*bool;
            fn request(p:*bool)->#relay *bool {{relay(p)#}}
            fn install(p:*bool)->*bool {{request(p)# {{relay(p)=>{body},}}}}
            fn run()->bool {{var flag=false;let p=install(flag.ref);p.* = true;flag}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            Value::Bool(true),
            "{body}"
        );
        let invalid = source.replace(
            "var flag=false;let p=install(flag.ref);p.* = true;flag",
            "let p=do {var flag=false;install(flag.ref)};p.*",
        );
        let package = support::compile(&invalid);
        let failure = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            failure.starts_with("cannot prepare resource-checked execution"),
            "{failure}"
        );
    }
}

#[test]
fn ordinary_pointer_delivery_snapshots_the_operand_before_later_arguments() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        effect relay(p:*bool,changed:bool)->*bool;
        fn run()->bool {var first=false;var second=false;var selected=first.ref;
            let p=relay(selected,do {selected=second.ref;true})# {relay(p,changed)=>p,};
            p.* = true;first and not second
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn effectful_callee_returns_original_pointer_without_extending_lifetime() {
    let declarations = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        effect poll()->bool;
        fn borrow(p:*bool)->#poll *bool {poll()#;p}";
    let source = format!(
        "{declarations}
        fn run()->bool {{var flag=false;
            let p=borrow(flag.ref)# {{poll()=>true,}};
            p.* = true;flag
        }}"
    );
    let package = support::compile(&source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000,)
            .unwrap(),
        Value::Bool(true)
    );
    let source = format!(
        "{declarations}
        fn run()->bool {{let p=do {{var flag=false;borrow(flag.ref)# {{poll()=>true,}}}};p.*}}"
    );
    let package = support::compile(&source);
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000,)
            .is_err()
    );
}

#[test]
fn effect_transfer_selectors_do_not_hide_unproved_calls_in_local_control() {
    for selector in [
        "do {let selected=decide(flag);selected}",
        "if flag {decide(flag)} else {false}",
        "flag match {true=>decide(flag),false=>false}",
        "do {decide(flag);flag}",
    ] {
        let source = format!(
            "effect relay(flag:bool,value:*bool)->*bool;
            fn decide(flag:bool)->bool {{flag}}
            fn install(flag:bool,value:*bool)->*bool {{
                relay(flag,value)# {{relay(flag,value)=>if ({selector}) {{value}} else {{value}},}}
            }}"
        );
        let package = support::compile(&source);
        let error = elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "install"),
            &[],
            &mut 200_000,
        )
        .unwrap_err();
        assert!(error.contains("data-only"), "{selector}: {error}");
    }
}

#[test]
fn effect_transfer_preserves_whole_aggregate_pattern_aliases() {
    for (declarations, ty, construct, arms, access) in [
        (
            "struct Link {flag:bool,target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{flag:true,target:flag.ref}",
            "relay(value)=>value match {{flag:true,target:_} as whole=>whole,{flag:false,target:_} as whole=>do {resume whole;}},",
            "value.target.*",
        ),
        (
            "",
            "Tuple<bool,*bool>",
            "(true,flag.ref)",
            "relay(value)=>value match {(_,_) as whole=>whole match {(true,_) as next=>next,(false,_) as next=>next}},",
            "value match {(_,p)=>p.*}",
        ),
        (
            "enum Choice {none,some(*bool)} impl Duplicate for Choice {}",
            "Choice",
            "Choice.some(flag.ref)",
            "relay(value)=>value match {(.some(_) as whole) or (.none as whole)=>whole},",
            "value match {.some(p)=>p.*,.none=>false}",
        ),
        (
            "struct Link {flag:bool,target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{flag:true,target:flag.ref}",
            "relay(value)=>if (value match {{flag:true,target:_}=>true,_=>false}) {value} else {value},",
            "value.target.*",
        ),
        (
            "struct Link {flag:bool,target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{flag:true,target:flag.ref}",
            "relay({flag:true,target:_} as whole)=>whole,relay({flag:false,target:_} as whole)=>whole,",
            "value.target.*",
        ),
        (
            "",
            "Tuple<bool,*bool>",
            "(true,flag.ref)",
            "relay((true,_) as whole)=>whole,relay((false,_) as whole)=>whole,",
            "value match {(_,p)=>p.*}",
        ),
        (
            "enum Choice {none,some(*bool)} impl Duplicate for Choice {}",
            "Choice",
            "Choice.some(flag.ref)",
            "relay(.some(_) as whole)=>whole,relay(.none as whole)=>whole,",
            "value match {.some(p)=>p.*,.none=>false}",
        ),
        (
            "enum Choice {none,some(*bool)} impl Duplicate for Choice {}",
            "Choice",
            "Choice.some(flag.ref)",
            "relay((.some(_) as whole) or (.none as whole))=>whole,",
            "value match {.some(p)=>p.*,.none=>false}",
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} typealias Payload={ty};effect relay(value:Payload)->Payload;
            fn install(value:Payload)->Payload {{relay(value)# {{{arms}}}}}"
        );
        let source = format!(
            "{prefix} fn run()->bool {{var flag=true;let value=install({construct});{access}}}"
        );
        let package = support::compile_with_tuple(&source);
        elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "run"),
            &[],
            &mut 300_000,
        )
        .unwrap_or_else(|error| panic!("{arms}: {error}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000
            )
            .unwrap_or_else(|error| panic!("{arms}: {error}")),
            Value::Bool(true)
        );
        let source = format!(
            "{prefix} fn run()->bool {{let value=do {{var flag=true;install({construct})}};{access}}}"
        );
        let package = support::compile_with_tuple(&source);
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            300_000,
        )
        .unwrap_err();
        assert!(
            error.starts_with("cannot prepare resource-checked execution"),
            "{arms}: {error}"
        );
    }
}

#[test]
fn effect_whole_record_pattern_preserves_owned_callback_consumption() {
    for handler in [
        "relay({flag:true,callback:_} as whole)=>whole,relay({flag:false,callback:_} as whole)=>do {resume whole;},",
        "relay(value)=>value match {{flag:true,callback:_} as whole=>whole,{flag:false,callback:_} as whole=>do {resume whole;}},",
        "relay(value)=>do {let {flag:_,callback:_} as whole=value;whole},",
        "relay(value)=>(do {let saved=value;saved}) match {{flag:_,callback:_} as whole=>whole},",
        "relay(value)=>do {let true=value.flag else {resume value;}; value},",
    ] {
        for flag in ["true", "false"] {
            for calls in [1, 2] {
                let call = if calls == 1 {
                    "bundle.callback()"
                } else {
                    "bundle.callback();bundle.callback()"
                };
                let source = format!(
                    "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                impl Duplicate for bool {{}}
                struct Ticket {{flag:bool}} impl Retire for Ticket {{fn drop(*self) {{}}}}
                fn take(ticket:Ticket)->bool {{ticket.flag}}
                typealias Callback=%{{ticket:Ticket}} fn()->bool;
                struct Bundle {{flag:bool,callback:Callback}}
                effect relay(value:Bundle)->Bundle;
                fn install(value:Bundle)->Bundle {{relay(value)# {{{handler}}}}}
                fn run()->bool {{let ticket=Ticket{{flag:{flag}}};
                    let bundle=install(Bundle{{flag:{flag},callback:|| take(ticket)}});{call}
                }}"
                );
                let package = support::compile(&source);
                let result = luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    300_000,
                );
                if calls == 1 {
                    assert_eq!(result.unwrap(), Value::Bool(flag == "true"));
                } else {
                    let error = result.unwrap_err();
                    assert!(
                        error.starts_with("cannot prepare resource-checked execution")
                            && error.contains("unavailable"),
                        "{error}"
                    );
                }
            }
        }
    }
}

#[test]
fn effect_aggregate_match_requires_producer_and_memory_evidence() {
    for (input, argument, declarations, body, reason) in [
        (
            "Link",
            "value",
            "fn produce(value:Link)->Link {value}",
            "produce(value) match {{flag:_,target:_} as whole=>whole}",
            "data-only",
        ),
        // Identity preservation does not authorize reading an unknown target.
        (
            "Link",
            "value",
            "",
            "value.target.* match {_=>value}",
            "unknown memory target",
        ),
    ] {
        let source=format!("struct Link {{flag:bool,target:*bool}} {declarations}
            effect relay({argument}:{input})->Link;
            fn install({argument}:{input})->Link {{relay({argument})# {{relay({argument})=>{body},}}}}");
        let package = support::compile(&source);
        let error = elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "install"),
            &[],
            &mut 300_000,
        )
        .unwrap_err();
        assert!(error.contains(reason), "{body}: {error}");
    }
}

#[test]
fn scalar_effect_delivery_can_ignore_copy_pointer_without_identity_transfer() {
    for body in ["true", "not false", "do {let flag=false;not flag}"] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            effect ignore(pointer:*bool)->bool;
            fn subject(pointer:*bool)->#ignore bool {{ignore(pointer)#}}
            fn run()->bool {{var flag=false;
                let result=subject(flag.ref)# {{ignore(_)=>{body},}};
                result and not flag
            }}"
        );
        let package = support::compile(&source);
        elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "run"),
            &[],
            &mut 200_000,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn owned_input_premise_cannot_authorize_pointer_results() {
    use elaboration::verification::memory;
    let package = support::compile(
        "effect relay(pointer:*bool)->*bool;
         fn subject(pointer:*bool)->#relay *bool {relay(pointer)#}",
    );
    let operation = &package.effects[0];
    let mut input = memory::EffectInput {
        parameter_accesses: Default::default(),
        operation: nir::effects::Operation {
            declaration: operation.declaration,
            arguments: vec![],
        },
        abi: operation.abi,
        signature: operation.signature.clone(),
        forwarded_parameters: Some(std::collections::BTreeSet::from([0])),
        owned_inputs: true,
        copy_parameters: Default::default(),
    };
    let infer = |input| {
        memory::infer_with_effect_inputs(
            &package,
            support::id(&package, "subject"),
            &[],
            &Default::default(),
            &Default::default(),
            &[input],
            &mut 200_000,
        )
    };
    let error = infer(input.clone()).unwrap_err();
    assert!(
        error.contains("owned effect inputs require direct delivery with a data-only result"),
        "{error}"
    );
    input.owned_inputs = false;
    assert!(infer(input).unwrap().result_targets.is_some());
}

#[test]
fn nonidentity_copy_effect_premises_require_checked_pointer_access() {
    use elaboration::verification::memory;
    let package = support::compile(
        "effect ignore(pointer:*bool)->bool;
        fn subject(pointer:*bool)->#ignore bool {ignore(pointer)#}",
    );
    let operation = &package.effects[0];
    let mut input = memory::EffectInput {
        parameter_accesses: Default::default(),
        operation: nir::effects::Operation {
            declaration: operation.declaration,
            arguments: vec![],
        },
        abi: operation.abi,
        signature: operation.signature.clone(),
        forwarded_parameters: None,
        owned_inputs: false,
        copy_parameters: std::collections::BTreeSet::from([0]),
    };
    let infer = |input: memory::EffectInput| {
        memory::infer_with_effect_inputs(
            &package,
            support::id(&package, "subject"),
            &[],
            &Default::default(),
            &Default::default(),
            &[input],
            &mut 200_000,
        )
    };
    let summary = infer(input.clone()).unwrap();
    assert!(summary.accesses.is_empty());
    assert!(summary.result_targets.is_none());
    input.copy_parameters.clear();
    assert!(infer(input.clone()).is_err());
    input.owned_inputs = true;
    assert!(format!("{input}").contains("(owned-inputs true)"));
    assert!(infer(input).is_ok());

    for body in ["pointer.*", "do {pointer.* = true;true}"] {
        let source = format!(
            "effect ignore(pointer:*bool)->bool;
            fn install(pointer:*bool)->bool {{ignore(pointer)# {{ignore(pointer)=>{body},}}}}"
        );
        let package = support::compile(&source);
        let summary = memory::infer_in_package(
            &package,
            support::id(&package, "install"),
            &[],
            &mut 200_000,
        )
        .unwrap();
        assert!(summary.accesses.iter().any(|access| access.read_value));
        assert_eq!(
            summary.accesses.iter().any(|access| access.write),
            body.contains("=")
        );
    }
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Link {target:*bool} impl Duplicate for Link {}
        impl Retire for Link {fn drop(*self) {}}
        effect ignore(value:Link)->bool;
        fn run()->bool {var flag=false;ignore(Link{target:flag.ref})# {ignore(_)=>true,}}",
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap_err();
    assert!(
        error.starts_with("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn conditional_effect_parameter_accesses_preserve_argument_snapshots() {
    use elaboration::verification::memory;
    use std::collections::BTreeSet;
    let package = support::compile(
        "effect touch(pointer:*bool,flag:bool)->bool;
        fn subject(left:*bool,right:*bool)->#touch bool {
            var selected=left;touch(selected,do {selected=right;true})#
        }",
    );
    let operation = &package.effects[0];
    let premise = |read_value, write| memory::EffectInput {
        operation: nir::effects::Operation {
            declaration: operation.declaration,
            arguments: vec![],
        },
        abi: operation.abi,
        signature: operation.signature.clone(),
        forwarded_parameters: None,
        owned_inputs: false,
        copy_parameters: BTreeSet::from([0]),
        parameter_accesses: BTreeSet::from([memory::EffectParameterAccess {
            parameter: 0,
            input_fields: vec![],
            read_value,
            write,
        }]),
    };
    let infer = |inputs: &[memory::EffectInput], fuel: &mut usize| {
        memory::infer_with_effect_inputs(
            &package,
            support::id(&package, "subject"),
            &[],
            &Default::default(),
            &Default::default(),
            inputs,
            fuel,
        )
    };
    for (read, write) in [(true, false), (false, true), (true, true)] {
        let input = premise(read, write);
        let summary = infer(&[input.clone()], &mut 200_000).unwrap();
        assert_eq!(summary.accesses.len(), 1);
        let access = &summary.accesses[0];
        assert_eq!(
            access.target.parameter, 0,
            "later pointer rebinding changed the prepared operand"
        );
        assert!(access.target.input_fields.is_empty());
        assert_eq!((access.read_value, access.write), (read, write));
        assert!(
            input
                .to_string()
                .contains("(parameter-access (parameter 0) (input-fields)")
        );
        assert!(infer(&[input.clone()], &mut 0).is_err());
        assert_eq!(summary, infer(&[input], &mut 200_000).unwrap());
    }
    for (parameter, fields, read, write) in [
        (2, vec![], true, false),
        (1, vec![], true, false),
        (0, vec![9], true, false),
        (0, vec![], false, false),
    ] {
        let mut input = premise(true, false);
        input.parameter_accesses = BTreeSet::from([memory::EffectParameterAccess {
            parameter,
            input_fields: fields,
            read_value: read,
            write,
        }]);
        assert!(infer(&[input], &mut 200_000).is_err());
    }
    let read = premise(true, false);
    let write = premise(false, true);
    for inputs in [[read.clone(), write.clone()], [write.clone(), read.clone()]] {
        let summary = infer(&inputs, &mut 200_000).unwrap();
        assert!(summary.accesses.iter().any(|access| access.read_value));
        assert!(summary.accesses.iter().any(|access| access.write));
        assert!(
            summary
                .accesses
                .iter()
                .all(|access| access.target.parameter == 0)
        );
    }
}

#[test]
fn conditional_effect_parameter_accesses_project_owned_input_fields() {
    use elaboration::verification::memory;
    let package = support::compile(
        "struct Link {target:*bool}
        effect touch(value:Link)->bool;
        fn subject(value:Link)->#touch bool {touch(value)#}",
    );
    let operation = &package.effects[0];
    let input = memory::EffectInput {
        operation: nir::effects::Operation {
            declaration: operation.declaration,
            arguments: vec![],
        },
        abi: operation.abi,
        signature: operation.signature.clone(),
        forwarded_parameters: None,
        owned_inputs: false,
        copy_parameters: std::collections::BTreeSet::from([0]),
        parameter_accesses: std::collections::BTreeSet::from([memory::EffectParameterAccess {
            parameter: 0,
            input_fields: vec![0],
            read_value: true,
            write: true,
        }]),
    };
    let summary = memory::infer_with_effect_inputs(
        &package,
        support::id(&package, "subject"),
        &[],
        &Default::default(),
        &Default::default(),
        &[input],
        &mut 200_000,
    )
    .unwrap();
    assert_eq!(summary.accesses.len(), 1);
    assert_eq!(summary.accesses[0].target.parameter, 0);
    assert_eq!(summary.accesses[0].target.input_fields, vec![0]);
    assert!(summary.accesses[0].read_value && summary.accesses[0].write);
}

#[test]
fn closed_handler_arms_infer_parameter_accesses_and_return_origins() {
    use elaboration::verification::memory;
    for (result, body, read, write, returns_pointer) in [
        ("bool", "pointer.*", true, false, false),
        ("bool", "do {pointer.* = true;false}", true, true, false),
        (
            "bool",
            "do {pointer.* = not pointer.*;pointer.*}",
            true,
            true,
            false,
        ),
        ("*bool", "do {pointer.* = true;pointer}", true, true, true),
        ("bool", "read(pointer)", true, false, false),
        ("bool", "do {resume pointer.*;}", true, false, false),
        ("*bool", "do {resume pointer;}", false, false, true),
        (
            "*bool",
            "if pointer.* {do {resume pointer;}} else {pointer}",
            true,
            false,
            true,
        ),
    ] {
        let package = support::compile(&format!(
            "fn read(pointer:*bool)->bool {{pointer.*}}
            effect touch(pointer:*bool)->{result};
            fn install(pointer:*bool)->{result} {{touch(pointer)# {{touch(pointer)=>{body},}}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "install")
            .unwrap();
        let mut handlers = Vec::new();
        nir::visit::expression(&function.body, &mut 200_000, |expr, _| {
            if let nir::program::ExprKind::EffectHandle {
                handlers: found, ..
            } = &expr.kind
            {
                handlers.extend(found.iter());
            }
            Ok(())
        })
        .unwrap();
        let handler = handlers[0];
        let infer = |fuel: &mut usize| {
            memory::infer_handler_arm_memory(&package, handler, &handler.arms[0], fuel)
        };
        assert!(infer(&mut 0).is_err());
        let summary = infer(&mut 200_000).unwrap_or_else(|error| panic!("{body}: {error}"));
        assert_eq!(
            summary.accesses.iter().any(|access| access.read_value),
            read,
            "{body}"
        );
        assert_eq!(
            summary.accesses.iter().any(|access| access.write),
            write,
            "{body}"
        );
        assert!(
            summary
                .accesses
                .iter()
                .all(|access| access.parameter == 0 && access.input_fields.is_empty())
        );
        if returns_pointer {
            assert_eq!(
                summary
                    .memory
                    .result_targets
                    .as_ref()
                    .unwrap()
                    .iter()
                    .next()
                    .unwrap()
                    .parameter,
                0
            );
        } else {
            assert!(summary.memory.result_targets.is_none());
        }
        assert!(summary.to_string().starts_with("(handler-parameter-memory"));
        assert_eq!(summary, infer(&mut 200_000).unwrap());
    }
}

#[test]
fn handler_parameter_inference_tracks_fields_and_rejects_missing_environments() {
    use elaboration::verification::memory;
    for (source, expected, fields, writes) in [
        ("struct Link {target:*bool} effect touch(value:Link)->bool;
          fn install(value:Link)->bool {touch(value)# {touch(value)=>do {value.target.* = true;value.target.*},}}", None, vec![0], true),
        ("effect touch(pointer:*bool)->bool;
          fn install(pointer:*bool,external:bool)->bool {touch(pointer)# {touch(pointer)=>external,}}", Some("capture environment"), vec![], false),
        ("effect touch(pointer:*bool)->bool;
          fn install(pointer:*bool)->bool {touch(pointer)# {touch(pointer)=>do {return pointer.*;},}}", None, vec![], false),
        ("effect touch(pointer:*bool)->bool;
          fn install(pointer:*bool)->*bool {touch(pointer)# {touch(pointer)=>do {return pointer;},};pointer}", Some("lexical return"), vec![], false),
    ] {
        let package=support::compile(source);
        let function=package.functions.iter().find(|function|function.name=="install").unwrap();
        let mut handlers=Vec::new();
        nir::visit::expression(&function.body,&mut 200_000,|expr,_| {
            if let nir::program::ExprKind::EffectHandle {handlers:found,..}=&expr.kind {handlers.extend(found.iter());}
            Ok(())
        }).unwrap();
        let result=memory::infer_handler_arm_memory(&package,handlers[0],&handlers[0].arms[0],&mut 200_000);
        if let Some(expected)=expected {
            let error=result.unwrap_err();assert!(error.contains(expected),"{error}");
        } else {
            let summary=result.unwrap();
            assert!(summary.accesses.iter().all(|access| access.parameter==0 && access.input_fields==fields));
            assert!(summary.accesses.iter().any(|access|access.read_value && access.write==writes));
            // A lexical scalar return evaluates its memory accesses but never
            // becomes the operation's ordinary pointer result. Pointer-valued
            // exits still need a separate exit summary, tested above.
            assert!(summary.memory.result_targets.is_none());
        }
    }
}

#[test]
fn ordinary_handlers_access_checked_scalar_pointer_parameters() {
    for (body, result, stored) in [
        ("pointer.*", false, false),
        ("do {pointer.* = true;false}", false, true),
        ("do {pointer.* = not pointer.*;pointer.*}", true, true),
        ("read(pointer)", false, false),
        ("do {resume pointer.*;}", false, false),
    ] {
        let prefix = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            fn read(pointer:*bool)->bool {{pointer.*}}
            effect touch(pointer:*bool)->bool;
            fn subject(pointer:*bool)->#touch bool {{touch(pointer)#}}
            fn install(pointer:*bool)->bool {{subject(pointer)# {{touch(pointer)=>{body},}}}}"
        );
        let source = format!(
            "{prefix} fn run()->bool {{var flag=false;let result=install(flag.ref);result=={result} and flag=={stored}}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                400_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            Value::Bool(true)
        );
        let source = format!(
            "{prefix} fn run()->bool {{let pointer=do {{var flag=false;flag.ref}};install(pointer)}}"
        );
        let package = support::compile(&source);
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            400_000,
        )
        .unwrap_err();
        assert!(
            error.starts_with("cannot prepare resource-checked execution"),
            "{body}: {error}"
        );
    }
}

#[test]
fn handler_parameter_access_preserves_request_snapshots_and_fallback() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}";
    let snapshot = format!(
        "{prefix}
        effect touch(pointer:*bool,flag:bool)->bool;
        fn subject(left:*bool,right:*bool)->#touch bool {{
            var selected=left;touch(selected,do {{selected=right;true}})#
        }}
        fn run()->bool {{var left=false;var right=false;
            subject(left.ref,right.ref)# {{touch(pointer,flag)=>do {{pointer.* = true;flag}},}};
            left and not right
        }}"
    );
    let package = support::compile(&snapshot);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 400_000)
            .unwrap(),
        Value::Bool(true)
    );
    for initial in ["true", "false"] {
        let source = format!(
            "{prefix}
            effect touch(pointer:*bool)->bool;
            fn run()->bool {{var flag={initial};
                let result=(touch(flag.ref)# {{touch(pointer) if pointer.* => false,}})# {{
                    touch(pointer)=>do {{pointer.* = true;true}},
                }};
                flag and result==not {initial}
            }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                400_000
            )
            .unwrap_or_else(|error| panic!("{initial}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn effect_parameter_writes_invalidate_pre_call_observations() {
    let source="effect touch(pointer:*bool)->bool;
        fn install(pointer:*bool)->bool {touch(pointer)# {touch(pointer)=>do {pointer.* = true;true},}}
        fn rejected(pointer:*bool)->result:bool where ensures result {
            let before=pointer.*;install(pointer);before==pointer.*
        }";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("postcondition"), "{error}");
}

#[test]
fn ordinary_handlers_access_aggregate_pointer_parameters() {
    for (declarations, ty, construct, body) in [
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{target:left.ref}",
            "value.target.* = true;true",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {} struct Pair {first:Link,second:Link} impl Duplicate for Pair {}",
            "Pair",
            "Pair{first:Link{target:left.ref},second:Link{target:right.ref}}",
            "value.first.target.* = true;value.second.target.* = false;true",
        ),
        (
            "impl Duplicate for Tuple<*bool,*bool> {}",
            "Tuple<*bool,*bool>",
            "(left.ref,right.ref)",
            "value match {(a,b)=>do {a.* = true;b.* = false;true}}",
        ),
        (
            "newtype Pointer=*bool;impl Duplicate for Pointer {}",
            "Pointer",
            "left.ref.as(Pointer)",
            "value.as(*bool).* = true;true",
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} typealias Payload={ty};
            effect touch(value:Payload)->bool;
            fn subject(value:Payload)->#touch bool {{touch(value)#}}
            fn install(value:Payload)->bool {{subject(value)# {{touch(value)=>do {{{body}}},}}}}"
        );
        let source = format!(
            "{prefix} fn run()->bool {{var left=false;var right=false;
            let result=install({construct});result and left and not right}}"
        );
        let package = support::compile_with_tuple(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                600_000
            )
            .unwrap_or_else(|error| panic!("{ty}: {error}")),
            Value::Bool(true)
        );
        let dangling = format!(
            "{prefix} fn run()->bool {{
            let value=do {{var left=false;var right=false;{construct}}};install(value)}}"
        );
        let package = support::compile_with_tuple(&dangling);
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                600_000
            )
            .is_err(),
            "{ty}: dangling input accepted"
        );
    }
}

#[test]
fn aggregate_handler_pointer_fields_may_alias() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Pair {first:*bool,second:*bool} impl Duplicate for Pair {}
        effect touch(value:Pair)->bool;
        fn run()->bool {var flag=false;
            let result=touch(Pair{first:flag.ref,second:flag.ref})# {
                touch(value)=>do {value.first.* = true;value.second.* = false;value.first.*},
            };not result and not flag
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 600_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn aggregate_handler_access_moves_affine_inputs_and_checks_copy_cleanup_compatibility() {
    for contract in [
        "",
        "impl Duplicate for Link {} impl Retire for Link {fn drop(*self) {}}",
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}} {contract}
            effect touch(value:Link)->bool;
            fn run()->bool {{var flag=false;touch(Link{{target:flag.ref}})# {{
                touch(value)=>do {{value.target.* = true;true}},
            }}}}"
        );
        let package = support::compile(&source);
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            600_000,
        );
        if contract.is_empty() {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert!(
                result.is_err(),
                "accepted incompatible Copy/Drop input: {contract}"
            );
        }
    }
}

#[test]
fn owned_handler_pointer_inputs_are_not_implicitly_copied() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Link {target:*bool}
        effect touch(value:Link);
        fn run() {
            var flag=false;
            let input=Link {target:flag.ref};
            touch(input)# {touch(value)=>{value.target.* = true;},};
            touch(input)# {touch(value)=>{value.target.* = false;},};
        }";
    let package = support::compile(source);
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 600_000)
            .unwrap_err();
    assert!(error.contains("unavailable value"), "{error}");
}

#[test]
fn aggregate_handler_access_uses_prepared_argument_snapshot() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Link {target:*bool} impl Duplicate for Link {}
        effect touch(value:Link,flag:bool)->bool;
        fn run()->bool {var left=false;var right=false;var selected=Link{target:left.ref};
            let result=touch(selected,do {selected=Link{target:right.ref};true})# {
                touch(value,flag)=>do {value.target.* = flag;true},
            };result and left and not right
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 600_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_handlers_destructure_pointer_parameters() {
    for (declarations, ty, construct, pattern, body) in [
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{target:flag.ref}",
            "{target:p}",
            "p.* = true;p.*",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {} struct Pair {first:Link,second:*bool} impl Duplicate for Pair {}",
            "Pair",
            "Pair{first:Link{target:flag.ref},second:other.ref}",
            "{first:{target:p},second:q}",
            "p.* = true;q.* = false;p.*",
        ),
        (
            "impl Duplicate for Tuple<*bool,*bool> {}",
            "Tuple<*bool,*bool>",
            "(flag.ref,other.ref)",
            "(p,q)",
            "p.* = true;q.* = false;p.*",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{target:flag.ref}",
            "{target:p} as whole",
            "p.* = true;whole.target.*",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{target:flag.ref}",
            "{target:p} or {target:p}",
            "p.* = true;p.*",
        ),
    ] {
        let prefix=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} typealias Payload={ty};effect touch(value:Payload)->bool;
            fn subject(value:Payload)->#touch bool {{touch(value)#}}
            fn install(value:Payload)->bool {{subject(value)# {{touch({pattern})=>do {{{body}}},}}}}");
        let source = format!(
            "{prefix} fn run()->bool {{var flag=false;var other=false;
            let result=install({construct});result and flag and not other}}"
        );
        let package = support::compile_with_tuple(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                600_000
            )
            .unwrap_or_else(|error| panic!("{pattern}: {error}")),
            Value::Bool(true)
        );
        let source = format!(
            "{prefix} fn run()->bool {{let value=do {{var flag=false;var other=false;{construct}}};install(value)}}"
        );
        let package = support::compile_with_tuple(&source);
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                600_000
            )
            .is_err(),
            "{pattern}: dangling target accepted"
        );
    }
}

#[test]
fn destructured_handler_guards_preserve_fallback_and_aliases() {
    for initial in ["true", "false"] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}} impl Duplicate for Link {{}}
            effect touch(value:Link,unused:bool)->bool;
            fn run()->bool {{var flag={initial};
                let result=(touch(Link{{target:flag.ref}},false)# {{
                    touch({{target:p}},_) if p.* => false,
                }})# {{touch({{target:p}},_)=>do {{p.* = true;resume true;}},}};
                flag and result==not {initial}
            }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                600_000
            )
            .unwrap_or_else(|error| panic!("{initial}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_parameter_access_uses_live_scalar_captures() {
    for (declarations, ty, construct, pattern, target) in [
        ("", "*bool", "flag.ref", "pointer", "pointer"),
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{target:flag.ref}",
            "{target:pointer}",
            "pointer",
        ),
    ] {
        let source=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} effect touch(value:{ty})->bool;
            fn twice(value:{ty})->#touch bool {{let first=touch(value)#;let second=touch(value)#;not first and second}}
            fn run()->bool {{var flag=false;var enabled=false;
                let result=twice({construct})# {{touch({pattern})=>do {{
                    {target}.* = enabled;enabled=not enabled;{target}.*
                }},}};
                result and flag and not enabled
            }}");
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("{ty}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_parameter_access_and_captured_guards_forward_together() {
    for enabled in ["true", "false"] {
        let source=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            effect touch(pointer:*bool)->bool;
            fn install(pointer:*bool,enabled:bool)->bool {{
                (touch(pointer)# {{touch(p) if enabled and not p.* => do {{p.* = true;resume true;}},}})# {{
                    touch(p)=>do {{p.* = true;false}},
                }}
            }}
            fn run()->bool {{var flag=false;let result=install(flag.ref,{enabled});flag and result=={enabled}}}");
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("{enabled}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_parameter_and_pointer_capture_accesses_keep_distinct_sources() {
    for aliased in [false, true] {
        let right = if aliased { "left.ref" } else { "right.ref" };
        let expected = if aliased {
            "not left and not right and not result"
        } else {
            "left and not right and result"
        };
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}} impl Duplicate for Link {{}}
            effect touch(value:Link)->bool;
            fn install(input:*bool,captured:*bool)->bool {{
                touch(Link{{target:input}})# {{touch({{target:p}})=>do {{
                    p.* = true;captured.* = false;p.*
                }},}}
            }}
            fn run()->bool {{var left=false;var right=false;
                let result=install(left.ref,{right});{expected}
            }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("aliased={aliased}: {error}")),
            Value::Bool(true)
        );
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "install"),
            &[],
            &mut 200_000,
        )
        .unwrap();
        for parameter in [0, 1] {
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|access| access.target.parameter == parameter && access.write),
                "parameter {parameter}: {summary:?}"
            );
        }
    }
}

#[test]
fn combined_handler_parameter_access_preserves_capture_restrictions() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        effect touch(pointer:*bool)->bool;";
    for body in [
        "var left=false;let captured=do {var right=false;right.ref};touch(left.ref)# {touch(p)=>do {p.* = true;captured.*},}",
        "var left=false;var right=false;var captured=right.ref;touch(left.ref)# {touch(p)=>do {captured=p;p.*},}",
        "var left=false;var enabled=false;let address=enabled.ref;touch(left.ref)# {touch(p)=>do {p.* = enabled;enabled=true;p.*},}",
    ] {
        let source = format!("{prefix} fn run()->bool {{{body}}}");
        let package = support::compile(&source);
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .is_err(),
            "accepted invalid capture: {body}"
        );
    }
}

#[test]
fn handler_input_borrows_do_not_reuse_captured_storage_roots() {
    let source = "typealias u32=.u32'builtin;trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for u32 {} impl Duplicate for *T where T {}
        effect touch(pointer:*bool)->bool;
        fn install(pointer:*bool,captured:*u32)->bool {
            touch(pointer)# {touch(p)=>do {p.* = true;captured.* = 7;p.*},}
        }
        fn run()->bool {var flag=false;var count:u32=0;
            let result=install(flag.ref,count.ref);result and flag and count==7
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn handlers_access_pointer_fields_in_captured_aggregates() {
    for (declarations, ty, construct, write, read) in [
        (
            "struct Link {target:*bool} impl Duplicate for Link {}",
            "Link",
            "Link{target:right.ref}",
            "captured.target.* = false",
            "captured.target.*",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {} struct Nested {link:Link} impl Duplicate for Nested {}",
            "Nested",
            "Nested{link:Link{target:right.ref}}",
            "captured.link.target.* = false",
            "captured.link.target.*",
        ),
        (
            "impl Duplicate for Tuple<*bool,*bool> {}",
            "Tuple<*bool,*bool>",
            "(right.ref,right.ref)",
            "captured match {(a,_)=>do {a.* = false;}}",
            "captured match {(_,b)=>b.*}",
        ),
        (
            "newtype Pointer=*bool;impl Duplicate for Pointer {}",
            "Pointer",
            "right.ref.as(Pointer)",
            "captured.as(*bool).* = false",
            "captured.as(*bool).*",
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} typealias Payload={ty};"
        );
        for parameter in [true, false] {
            let (effect, request, pattern, body) = if parameter {
                (
                    "effect touch(pointer:*bool)->bool;",
                    "touch(input)",
                    "touch(p)",
                    format!("p.* = true;{write};p.*"),
                )
            } else {
                (
                    "effect touch()->bool;",
                    "touch()",
                    "touch()",
                    format!("{write};not ({read})"),
                )
            };
            let source=format!("{prefix}{effect}
                fn install(input:*bool,captured:Payload)->bool {{
                    {request}# {{{pattern}=>do {{{body}}},}}
                }}
                fn run()->bool {{var left=false;var right=true;
                    let result=install(left.ref,{construct});result and left=={parameter} and not right
                }}");
            let package = support::compile_with_tuple(&source);
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    800_000
                )
                .unwrap_or_else(|error| panic!("{ty} parameter={parameter}: {error}")),
                Value::Bool(true)
            );
            let summary = elaboration::verification::memory::infer_in_package(
                &package,
                support::id(&package, "install"),
                &[],
                &mut 200_000,
            )
            .unwrap();
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|access| access.target.parameter == 1 && access.write),
                "{ty}: captured write lost"
            );
        }
    }
}

#[test]
fn handler_aggregate_captures_require_stable_live_storage() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Link {target:*bool} impl Duplicate for Link {}
        effect touch()->bool;";
    for (setup, mutation) in [
        (
            "let captured=do {var flag=false;Link{target:flag.ref}};",
            "",
        ),
        (
            "var flag=false;var other=false;var captured=Link{target:flag.ref};",
            "captured=Link{target:other.ref};",
        ),
        (
            "var flag=false;var other=false;var captured=Link{target:flag.ref};",
            "captured.target=other.ref;",
        ),
        (
            "var flag=false;let captured=Link{target:flag.ref};let exposed=captured.ref;",
            "",
        ),
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{{setup}touch()# {{touch()=>do {{{mutation}captured.target.* = true;captured.target.*}},}}}}"
        );
        let package = support::compile(&source);
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .is_err(),
            "accepted unstable capture: {setup} {mutation}"
        );
    }
}

#[test]
fn aggregate_capture_aliases_share_actual_storage_with_request_inputs() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Pair {first:*bool,second:*bool} impl Duplicate for Pair {}
        effect touch(pointer:*bool)->bool;";
    let source = format!(
        "{prefix} fn run()->bool {{var flag=false;
        let captured=Pair{{first:flag.ref,second:flag.ref}};
        let result=touch(flag.ref)# {{touch(p)=>do {{
            p.* = true;captured.first.* = false;captured.second.*
        }},}};not result and not flag
    }}"
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800_000)
            .unwrap(),
        Value::Bool(true)
    );
    let with_drop = format!("impl Retire for Pair {{fn drop(*self) {{}}}} {source}");
    let package = support::compile(&with_drop);
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800_000)
            .is_err(),
        "conflicting cleanup accepted for captured aggregate"
    );
}

#[test]
fn handler_pointer_aggregate_captures_update_disjoint_data_fields() {
    for (declarations, construct, pointer, next, update) in [
        (
            "struct State {target:*bool,next:bool} impl Duplicate for State {}",
            "State{target:flag.ref,next:false}",
            "state.target",
            "state.next",
            "state.next=not state.next",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {} struct Flags {next:bool} impl Duplicate for Flags {} struct State {link:Link,flags:Flags} impl Duplicate for State {}",
            "State{link:Link{target:flag.ref},flags:Flags{next:false}}",
            "state.link.target",
            "state.flags.next",
            "state.flags.next=not state.flags.next",
        ),
        (
            "struct Link {target:*bool} impl Duplicate for Link {} struct Flags {next:bool} impl Duplicate for Flags {} struct State {link:Link,flags:Flags} impl Duplicate for State {}",
            "State{link:Link{target:flag.ref},flags:Flags{next:false}}",
            "state.link.target",
            "state.flags.next",
            "state.flags=Flags{next:not state.flags.next}",
        ),
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            {declarations} effect touch()->bool;
            fn twice()->#touch bool {{let first=touch()#;let second=touch()#;not first and second}}
            fn run()->bool {{var flag=false;var state={construct};
                let result=twice()# {{touch()=>do {{
                    {pointer}.* = {next};{update};{pointer}.*
                }},}};result and flag and not {next}
            }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("{update}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_pointer_capture_stability_rejects_ancestor_writes_and_addresses() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Link {target:*bool} impl Duplicate for Link {}
        struct State {link:Link,flag:bool} impl Duplicate for State {}
        effect touch()->bool;";
    for mutation in [
        "state.link=Link{target:other.ref};",
        "state.link.target=other.ref;",
        "let exposed=state.flag.ref;",
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{var flag=false;var other=false;
            var state=State{{link:Link{{target:flag.ref}},flag:false}};
            touch()# {{touch()=>do {{{mutation}state.flag=true;state.link.target.*}},}}
        }}"
        );
        let package = support::compile(&source);
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .is_err(),
            "accepted unstable pointer capture: {mutation}"
        );
    }
}

#[test]
fn nested_handlers_preserve_pointer_field_stability_with_mutable_data() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct State {target:*bool,next:bool} impl Duplicate for State {}
        effect start()->bool; effect touch()->bool;
        fn twice()->#touch bool {let first=touch()#;let second=touch()#;not first and second}
        fn run()->bool {start()# {start()=>do {
            var flag=false;var state=State{target:flag.ref,next:false};
            let result=twice()# {touch()=>do {
                state.target.* = state.next;state.next=not state.next;state.target.*
            },};result and flag and not state.next
        },}}
    ";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800_000)
            .unwrap(),
        Value::Bool(true)
    );
    let stationary = source.replace("state.next=not state.next;", "state.target=state.target;");
    let package = support::compile(&stationary);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800_000)
            .unwrap(),
        Value::Bool(false),
    );
    let unstable = source.replace(
        "state.next=not state.next;",
        "var other=false;state.target=other.ref;",
    );
    let package = support::compile(&unstable);
    assert_capture_writeback_rejected(&package);
}

#[test]
fn effect_variant_parameter_access_checks_only_present_payloads() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        enum Choice {none,some(*bool)} impl Duplicate for Choice {}
        effect touch(value:Choice)->bool;
        fn install(value:Choice)->bool {touch(value)# {
            touch(.none)=>false,
            touch(.some(p))=>do {p.* = true;p.*},
        }}";
    for (value, expected) in [
        ("Choice.none", "not result and not flag"),
        ("Choice.some(flag.ref)", "result and flag"),
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{var flag=false;let result=install({value});{expected}}}"
        );
        let package = support::compile(&source);
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "install")
            .unwrap();
        nir::visit::expression(&function.body, &mut 200_000, |expr, fuel| {
            if let nir::program::ExprKind::EffectHandle { handlers, .. } = &expr.kind {
                let summary = elaboration::verification::memory::infer_handler_arm_memory(
                    &package,
                    &handlers[0],
                    &handlers[0].arms[1],
                    fuel,
                )?;
                assert!(
                    summary
                        .accesses
                        .iter()
                        .any(|access| access.to_string().contains("(variant 1)"))
                );
                assert!(
                    summary
                        .accesses
                        .iter()
                        .all(|access| access.input_fields == [usize::MAX, 1, 0])
                );
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("{value}: {error}")),
            Value::Bool(true)
        );
    }
    let source = format!(
        "{prefix} fn run()->bool {{let value=do {{var flag=false;Choice.some(flag.ref)}};install(value)}}"
    );
    let package = support::compile(&source);
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800_000)
            .is_err(),
        "present dangling payload accepted"
    );
}

#[test]
fn effect_variant_access_preserves_prepared_tags_and_pointer_origins() {
    for present in [true, false] {
        let (initial, later, expected) = if present {
            ("Choice.some(flag.ref)", "Choice.none", "result and flag")
        } else {
            (
                "Choice.none",
                "Choice.some(flag.ref)",
                "not result and not flag",
            )
        };
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            enum Choice {{none,some(*bool)}} impl Duplicate for Choice {{}}
            effect touch(value:Choice,flag:bool)->bool;
            fn run()->bool {{var flag=false;var chosen={initial};
                let result=touch(chosen,do {{chosen={later};true}})# {{
                    touch(.none,_)=>false,
                    touch(.some(p),value)=>do {{p.* = value;p.*}},
                }};{expected}
            }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("present={present}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn effect_variant_arms_keep_overlapping_payload_types_separate() {
    let prefix = "typealias u32=.u32'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for u32 {} impl Duplicate for *T where T {}
        enum Choice {flag(*bool),count(*u32)} impl Duplicate for Choice {}
        effect touch(value:Choice)->bool;
        fn install(value:Choice)->bool {touch(value)# {
            touch(.flag(p))=>do {p.* = true;p.*},
            touch(.count(p))=>do {p.* = 7;p.* == 7},
        }}";
    for projected in [false, true] {
        let prefix = if projected {
            prefix.replace("fn install(value:Choice)","struct Packet {value:Choice} impl Duplicate for Packet {} fn install(packet:Packet)").replace("touch(value)#", "touch(packet.value)#")
        } else {
            prefix.to_owned()
        };
        for (value, expected) in [
            ("Choice.flag(flag.ref)", "flag and count==0"),
            ("Choice.count(count.ref)", "not flag and count==7"),
        ] {
            let value = if projected {
                format!("Packet{{value:{value}}}")
            } else {
                value.to_owned()
            };
            let source = format!(
                "{prefix} fn run()->bool {{var flag=false;var count:u32=0;let result=install({value});result and ({expected})}}"
            );
            let package = support::compile(&source);
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    800_000
                )
                .unwrap_or_else(|error| panic!("{value}: {error}")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn effect_whole_variant_match_preserves_possible_pointer_sources() {
    for tag in ["left", "right"] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            enum Choice {{left(*bool),right(*bool)}} impl Duplicate for Choice {{}}
            effect touch(value:Choice)->bool;
            fn run()->bool {{var flag=false;let result=touch(Choice.{tag}(flag.ref))# {{
                touch(value)=>value match {{
                    .left(p)=>do {{p.* = true;p.*}},
                    .right(p)=>do {{p.* = true;p.*}},
                }},
            }};result and flag}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("{tag}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_variant_captures_preserve_absent_and_present_payloads() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        enum Choice {none,some(*bool)} impl Duplicate for Choice {}
        effect touch()->bool;
        fn install(captured:Choice)->bool {touch()# {touch()=>captured match {
            .none=>false,.some(p)=>do {p.* = true;p.*},
        },}}";
    for (value, expected) in [
        ("Choice.none", "not result and not flag"),
        ("Choice.some(flag.ref)", "result and flag"),
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{var flag=false;let result=install({value});{expected}}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800_000
            )
            .unwrap_or_else(|error| panic!("{value}: {error}")),
            Value::Bool(true)
        );
    }
    let source = format!(
        "{prefix} fn run()->bool {{let value=do {{var flag=false;Choice.some(flag.ref)}};install(value)}}"
    );
    let package = support::compile(&source);
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800_000)
            .is_err()
    );
}

#[test]
fn handler_nested_variant_captures_preserve_guard_paths() {
    for (declarations, ty, construct, body, present) in [
        (
            "struct Packet {target:Choice} impl Duplicate for Packet {}",
            "Packet",
            "Packet{target:Choice.none}",
            "captured.target match {.none=>false,.some(p)=>do {p.* = true;p.*}}",
            false,
        ),
        (
            "struct Packet {target:Choice} impl Duplicate for Packet {}",
            "Packet",
            "Packet{target:Choice.some(flag.ref)}",
            "captured.target match {.none=>false,.some(p)=>do {p.* = true;p.*}}",
            true,
        ),
        (
            "enum Outer {none,some(Choice)} impl Duplicate for Outer {}",
            "Outer",
            "Outer.some(Choice.none)",
            "captured match {.none=>false,.some(.none)=>false,.some(.some(p))=>do {p.* = true;p.*}}",
            false,
        ),
        (
            "enum Outer {none,some(Choice)} impl Duplicate for Outer {}",
            "Outer",
            "Outer.some(Choice.some(flag.ref))",
            "captured match {.none=>false,.some(.none)=>false,.some(.some(p))=>do {p.* = true;p.*}}",
            true,
        ),
    ] {
        let source=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            enum Choice {{none,some(*bool)}} impl Duplicate for Choice {{}}
            {declarations} effect touch()->bool;
            fn install(captured:{ty})->bool {{touch()# {{touch()=>{body},}}}}
            fn run()->bool {{var flag=false;let result=install({construct});result=={present} and flag=={present}}}");
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                900_000
            )
            .unwrap_or_else(|error| panic!("{construct}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_variant_capture_does_not_freeze_unrelated_mutable_tags() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        enum Target {none,some(*bool)} impl Duplicate for Target {}
        enum Mode {off,on} impl Duplicate for Mode {}
        struct State {target:Target,mode:Mode} impl Duplicate for State {}
        effect touch()->bool;
        fn twice()->#touch bool {let first=touch()#;let second=touch()#;not first and second}
        fn run()->bool {var flag=false;var state=State{target:Target.some(flag.ref),mode:Mode.off};
            let result=twice()# {touch()=>do {
                let next=state.mode match {.off=>false,.on=>true};
                state.mode=Mode.on;
                state.target match {.none=>false,.some(p)=>do {p.* = next;p.*}}
            },};result and flag
        }";
    for present in [true, false] {
        let source = source.replace(
            "result and flag",
            &format!("result=={present} and flag=={present}"),
        );
        let source = if present {
            source
        } else {
            source.replace("Target.some(flag.ref)", "Target.none")
        };
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                900_000
            )
            .unwrap_or_else(|error| panic!("present={present}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_variant_capture_rejects_tag_rebinding_even_when_initially_empty() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        enum Choice {none,some(*bool)} impl Duplicate for Choice {}
        effect touch()->bool;";
    for before in [
        "captured=Choice.some(flag.ref);",
        "let exposed=captured.ref;",
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{var flag=false;var captured=Choice.none;
            touch()# {{touch()=>do {{{before} captured match {{.none=>false,.some(p)=>p.*}}}},}}
        }}"
        );
        let package = support::compile(&source);
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                900_000
            )
            .is_err(),
            "tag stability was not checked: {before}"
        );
    }
}

#[test]
fn handler_known_variant_capture_separates_different_payload_types() {
    for (construct, expected) in [
        ("Choice.flag(flag.ref)", "flag and count==0"),
        ("Choice.count(count.ref)", "not flag and count==7"),
    ] {
        let source=format!("typealias u32=.u32'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for u32 {{}} impl Duplicate for *T where T {{}}
            enum Choice {{flag(*bool),count(*u32)}} impl Duplicate for Choice {{}}
            effect touch()->bool;
            fn run()->bool {{var flag=false;var count:u32=0;let captured={construct};
                let result=touch()# {{touch()=>captured match {{
                    .flag(p)=>do {{p.* = true;p.*}},
                    .count(p)=>do {{p.* = 7;p.* == 7}},
                }},}};result and ({expected})
            }}");
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                900_000
            )
            .unwrap_or_else(|error| panic!("{construct}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn handler_matches_separate_unknown_heterogeneous_payload_origins() {
    for capture in [false, true] {
        let (effect, request, pattern, subject) = if capture {
            ("effect touch()->bool;", "touch()", "touch()", "value")
        } else {
            (
                "effect touch(value:Choice)->bool;",
                "touch(value)",
                "touch(input)",
                "input",
            )
        };
        let prefix=format!("typealias u32=.u32'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for u32 {{}} impl Duplicate for *T where T {{}}
            enum Choice {{flag(*bool),count(*u32)}} impl Duplicate for Choice {{}}
            {effect}
            fn install(value:Choice)->bool {{{request}# {{{pattern}=>{subject} match {{
                .flag(p)=>do {{p.* = true;p.*}},
                .count(p)=>do {{p.* = 7;p.* == 7}},
            }},}}}}");
        for (value, expected) in [
            ("Choice.flag(flag.ref)", "flag and count==0"),
            ("Choice.count(count.ref)", "not flag and count==7"),
        ] {
            let source = format!(
                "{prefix} fn run()->bool {{var flag=false;var count:u32=0;
                let result=install({value});result and ({expected})}}"
            );
            let package = support::compile(&source);
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .unwrap_or_else(|error| panic!("capture={capture} {value}: {error}")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn variant_matches_route_qualified_pointers_to_typed_effect_requests() {
    for (setup, subject) in [
        ("", "value"),
        ("let alias=value;", "alias"),
        ("let _ as alias=value;", "alias"),
    ] {
        let prefix = "typealias u32=.u32'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for u32 {} impl Duplicate for *T where T {}
        enum Choice {flag(*bool),count(*u32)} impl Duplicate for Choice {}
        effect set_flag(p:*bool)->bool;effect set_count(p:*u32)->bool;
        fn install(value:Choice)->bool {value match {
            .flag(p)=>set_flag(p)# {set_flag(q)=>do {q.* = true;q.*},},
            .count(p)=>set_count(p)# {set_count(q)=>do {q.* = 7;q.* == 7},},
        }}";
        for (value, expected) in [
            ("Choice.flag(flag.ref)", "flag and count==0"),
            ("Choice.count(count.ref)", "not flag and count==7"),
        ] {
            let prefix = prefix.replace("{value match", &format!("{{{setup}{subject} match"));
            let source = format!(
                "{prefix} fn run()->bool {{var flag=false;var count:u32=0;
            let result=install({value});result and ({expected})}}"
            );
            let package = support::compile(&source);
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .unwrap_or_else(|error| panic!("{value}: {error}")),
                Value::Bool(true)
            );
        }
    }
}
#[test]
fn effect_aliases_preserve_variant_conditions_until_storage_is_replaced() {
    let seed = support::compile_with_tuple(
        "typealias u32=.u32'builtin;trait Duplicate {}trait Retire {fn drop(*self);}",
    );
    let protocols = elaboration::program::Protocols {
        copy: Some(
            seed.traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            seed.traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    for (setup, replaced) in [
        ("let prepared=value;", false),
        (
            "let wrapped=choose_envelope(other.* == 1,Envelope.none,Envelope.slot(value))# {choose_envelope(flag,left,right)=>if flag {left} else {right},};let .slot(prepared)=wrapped else {return false;};",
            false,
        ),
        (
            "var current=Envelope.slot(value);let wrapped=choose_envelope(other.* == 0,current,do {current=Envelope.none;Envelope.none})# {choose_envelope(flag,left,right)=>if flag {left} else {right},};let .slot(prepared)=wrapped else {return false;};",
            false,
        ),
        (
            "let wrapped=choose_envelope(other.* == 1,Envelope.none,Envelope.slot(Choice.count(other)))# {choose_envelope(flag,left,right)=>if flag {left} else {right},};let .slot(prepared)=wrapped else {return false;};",
            true,
        ),
        (
            "let wrapped=choose_envelope(other.* == 0,Envelope.slot(value),Envelope.none)# {choose_envelope(flag,left,right)=>if flag {left} else {right},};let .slot(prepared)=wrapped else {return false;};",
            false,
        ),
        (
            "let wrapped=if other.* == 1 {Envelope.none} else {Envelope.slot(value)};let loaded=wrapped.ref.*;let .slot(prepared)=loaded else {return false;};",
            false,
        ),
        (
            "let wrapped=(other.* == 0) match {true=>Envelope.slot(value),false=>Envelope.none,};let loaded=wrapped.ref.*;let .slot(prepared)=loaded else {return false;};",
            false,
        ),
        (
            "let first=Envelope.slot(value);let second=Envelope.none;let loaded=do {let pointer=if other.* == 0 {first.ref} else {second.ref};pointer.*};let .slot(prepared)=loaded else {return false;};",
            false,
        ),
        (
            "let wrapped=Envelope.slot(value);let loaded=wrapped.ref.*;let .slot(prepared)=loaded else {return false;};",
            false,
        ),
        (
            "let wrapped=if other.* == 0 {Envelope.slot(value)} else {Envelope.none};let loaded=wrapped.ref.*;let .slot(prepared)=loaded else {return false;};",
            false,
        ),
        ("let prepared=load_local(value);", false),
        ("let prepared=do {let load=||value.ref.*;load()};", false),
        (
            "let prepared=do {let load=||load_local(value);load()};",
            false,
        ),
        ("let prepared=load_local(Choice.count(other));", true),
        ("let prepared=do {let pointer=value.ref;pointer.*};", false),
        (
            "let holder=Holder {inner:value};let prepared=holder.inner.ref.*;",
            false,
        ),
        (
            "var current=value;let prepared=do {let pointer=current.ref;pointer.*};current=Choice.count(other);",
            false,
        ),
        (
            "var current=value;let prepared=do {let pointer=current.ref;current=Choice.count(other);pointer.*};",
            true,
        ),
        (
            "let first=value;let second=Choice.count(other);let prepared=do {let pointer=if other.* == 0 {first.ref} else {second.ref};pointer.*};",
            false,
        ),
        (
            "let first=value;let second=Choice.count(other);let prepared=do {let pointer=if other.* == 0 {second.ref} else {first.ref};pointer.*};",
            true,
        ),
        (
            "let prepared=default_relay(value,.copy=Choice.count(other));",
            true,
        ),
        (
            "let prepared=default_wrap(Choice.count(other)).inner;",
            true,
        ),
        ("let prepared=default_relay(value);", false),
        ("let prepared=default_wrap(value).inner;", false),
        ("let prepared=default_call(value);", false),
        (
            "var current=value;let prepared=default_after(current,do {current=Choice.count(other);false});",
            false,
        ),
        (
            "let prepared=relay_value(value)# {relay_value(v)=>v,};",
            false,
        ),
        (
            "let prepared=relay_value(value)# {relay_value(v)=>do {resume v;},};",
            false,
        ),
        (
            "let prepared=choose_value(other.* == 0,value,Choice.count(other))# {choose_value(f,l,r)=>if f {l} else {r},};",
            false,
        ),
        (
            "let prepared=choose_value(other.* != 0,value,Choice.count(other))# {choose_value(f,l,r)=>if f {l} else {r},};",
            true,
        ),
        (
            "var current=value;let prepared=relay_later(current,do {current=Choice.count(other);false})# {relay_later(v,_)=>v,};",
            false,
        ),
        (
            "let prepared=(relay_value(value)# {relay_value(v) if false=>v,})# {relay_value(v)=>v,};",
            false,
        ),
        ("let make=|| value;make();let prepared=make();", false),
        (
            "var captured=value;let make=|| {let old=captured;captured=Choice.count(other);old};make();let prepared=make();",
            true,
        ),
        (
            "var captured=value;let make=|| {if other.* == 0 {return captured;}captured=Choice.count(other);captured};make();let prepared=make();",
            false,
        ),
        ("let make=|v:Choice| v;let prepared=make(value);", false),
        (
            "let prepared=(|v:Choice| Holder {inner:v})(value).inner;",
            false,
        ),
        ("let make=|| value;let prepared=make();", false),
        ("let prepared=(|| value)();", false),
        (
            "var current=value;let make=|| current;current=Choice.count(other);let prepared=make();",
            false,
        ),
        (
            "let make=if other.* == 0 {|v:Choice,p:*u32| v} else {|v:Choice,p:*u32| Choice.count(p)};let prepared=make(value,other);",
            false,
        ),
        (
            "let make=if other.* == 0 {|v:Choice,p:*u32| Choice.count(p)} else {|v:Choice,p:*u32| v};let prepared=make(value,other);",
            true,
        ),
        ("let make=|| Choice.count(other);let prepared=make();", true),
        (
            "let route=if other.* == 0 {keep} else {replace};let prepared=route(value,other);",
            false,
        ),
        (
            "let route=if other.* == 0 {replace} else {keep};let prepared=route(value,other);",
            true,
        ),
        (
            "let prepared=(if other.* == 0 {keep} else {replace})(value,other);",
            false,
        ),
        (
            "let prepared=(if other.* == 0 {replace} else {keep})(value,other);",
            true,
        ),
        (
            "var current=value;let route=if other.* == 0 {relay_after} else {relay_after_alt};let prepared=route(current,do {current=Choice.count(other);false});",
            false,
        ),
        ("let prepared=relay(value);", false),
        ("let prepared=wrap(value).inner;", false),
        (
            "let prepared=relay_wrapped(value.as(Wrapped)).as(Choice);",
            false,
        ),
        ("let prepared=relay_recursive(false,value);", false),
        ("let prepared=select(value,other,other.* == 0);", false),
        ("let prepared=select(value,other,other.* != 0);", true),
        (
            "var current=value;let prepared=relay_after(current,do {current=Choice.count(other);false});",
            false,
        ),
        ("let prepared=(Holder {inner:value}).inner;", false),
        (
            "let prepared=(Nested {holder:Holder {inner:value}}).holder.inner;",
            false,
        ),
        (
            "let prepared=((Holder {inner:value}).as(WrappedHolder)).as(Holder).inner;",
            false,
        ),
        (
            "var current=value;let prepared=(Pair {inner:current,flag:do {current=Choice.count(other);false}}).inner;",
            false,
        ),
        (
            "let prepared=(if other.* == 0 {Holder {inner:value}} else {Holder {inner:value}}).inner;",
            false,
        ),
        (
            "let prepared=((other.* == 0) match {true=>Holder {inner:value},false=>Holder {inner:value},}).inner;",
            false,
        ),
        (
            "let prepared=(Holder {inner:Choice.count(other)}).inner;",
            true,
        ),
        ("let prepared=if other.* == 0 {value} else {value};", false),
        (
            "let prepared=(other.* == 0) match {true=>value,false=>value,};",
            false,
        ),
        (
            "let prepared=if other.* == 0 {value} else {return false;};",
            false,
        ),
        (
            "let prepared=if other.* == 0 {value} else {Choice.count(other)};",
            false,
        ),
        (
            "let prepared=if other.* == 0 {Choice.count(other)} else {value};",
            true,
        ),
        (
            "let prepared=(other.* == 0) match {true=>Choice.count(other),false=>value,};",
            true,
        ),
        (
            "let holder=Holder {inner:value};let prepared=holder.inner;",
            false,
        ),
        ("let (prepared,_)=(value,false);", false),
        (
            "let wrapped=value.as(Wrapped);let prepared=wrapped.as(Choice);",
            false,
        ),
        (
            "let .slot(prepared)=Envelope.slot(value) else {return false;};",
            false,
        ),
        (
            "var current=value;let pair=Pair {inner:current,flag:do {current=Choice.count(other);false}};let prepared=pair.inner;",
            false,
        ),
        (
            "let prepared=do {let holder=Holder {inner:value};holder.inner};",
            false,
        ),
        ("var prepared=Choice.count(other);prepared=value;", false),
        ("var prepared=value;prepared=prepared;", false),
        (
            "var holder=Holder {inner:Choice.count(other)};holder.inner=value;let prepared=holder.inner;",
            false,
        ),
        (
            "var prepared=value;if other.* == 0 {prepared=value;} else {prepared=value;}",
            false,
        ),
        (
            "var current=value;let prepared=current;current=Choice.count(other);",
            false,
        ),
        ("var prepared=value;prepared=Choice.count(other);", true),
        (
            "var holder=Holder {inner:value};holder.inner=Choice.count(other);let prepared=holder.inner;",
            true,
        ),
    ] {
        let prefix=format!("typealias u32=.u32'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for u32 {{}} impl Duplicate for *T where T {{}}
            enum Choice {{flag(*bool),count(*u32)}} impl Duplicate for Choice {{}}
            struct Holder {{inner:Choice}} impl Duplicate for Holder {{}}
            struct Nested {{holder:Holder}} impl Duplicate for Nested {{}}
            newtype WrappedHolder=Holder; impl Duplicate for WrappedHolder {{}}
            struct Pair {{inner:Choice,flag:bool}} impl Duplicate for Pair {{}}
            impl Duplicate for Tuple<Choice,bool> {{}}
            newtype Wrapped=Choice; impl Duplicate for Wrapped {{}}
            enum Envelope {{none,slot(Choice)}} impl Duplicate for Envelope {{}}
            fn keep(value:Choice,other:*u32)->Choice {{value}}
            fn replace(value:Choice,other:*u32)->Choice {{Choice.count(other)}}
            fn relay_after_alt(value:Choice,flag:bool)->Choice {{relay(value)}}
            fn default_relay(value:Choice,.copy:Choice=value)->Choice {{copy}}
            fn default_wrap(value:Choice,.copy:Choice=value,.wrapped:Holder=Holder {{inner:copy}})->Holder {{wrapped}}
            pure fn default_identity(value:Choice)->Choice {{value}}
            fn default_call(value:Choice,.copy:Choice=default_identity(value))->Choice {{copy}}
            fn default_after(value:Choice,flag:bool,.copy:Choice=value)->Choice {{copy}}
            fn relay(value:Choice)->Choice {{value}}
            fn load_local(value:Choice)->Choice {{value.ref.*}}
            fn wrap(value:Choice)->Holder {{Holder {{inner:value}}}}
            fn relay_wrapped(value:Wrapped)->Wrapped {{value}}
            fn relay_recursive(stop:bool,value:Choice)->Choice {{if stop {{value}} else {{relay_recursive(true,value)}}}}
            fn select(value:Choice,other:*u32,flag:bool)->Choice {{if flag {{value}} else {{Choice.count(other)}}}}
            fn relay_after(value:Choice,flag:bool)->Choice {{value}}
            effect relay_value(value:Choice)->Choice;
            effect relay_later(value:Choice,flag:bool)->Choice;
            effect choose_value(flag:bool,left:Choice,right:Choice)->Choice;
            effect choose_envelope(flag:bool,left:Envelope,right:Envelope)->Envelope;
            effect touch(value:Choice)->bool;
            fn install(value:Choice,other:*u32)->bool {{{setup} touch(prepared)# {{
                touch(.flag(p))=>do {{p.* = true;p.*}},
                touch(.count(p))=>do {{p.* = 7;p.* == 7}},
            }}}}");
        for (value, original) in [
            ("Choice.flag(flag.ref)", "flag and count==0"),
            ("Choice.count(count.ref)", "not flag and count==7"),
        ] {
            let expected = if replaced {
                "not flag and count==0 and other==7".to_owned()
            } else {
                format!("{original} and other==0")
            };
            let source = format!(
                "{prefix} fn run()->bool {{var flag=false;var count:u32=0;var other:u32=0;
                let result=install({value},other.ref);result and ({expected})}}"
            );
            let package = support::compile_with_tuple_protocols(&source, &protocols)
                .unwrap_or_else(|error| panic!("{setup}: {error}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .unwrap_or_else(|error| panic!("{setup} {value}: {error}")),
                Value::Bool(true)
            );
        }
        let source = format!(
            "{prefix} fn run()->bool {{var flag=false;let other=do {{var count:u32=0;count.ref}};
            install(Choice.flag(flag.ref),other)}}"
        );
        if replaced {
            let package = support::compile_with_tuple_protocols(&source, &protocols)
                .unwrap_or_else(|error| panic!("{setup}: {error}"));
            assert!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .is_err(),
                "replacement hid a dead target"
            );
        }
    }
}

#[test]
fn closure_capture_guard_exits_have_structured_dumps() {
    let package = support::compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        enum Choice {first(*bool),second(*bool)}impl Duplicate for Choice {}
        fn run()->bool {let flag=true;let value=Choice.first(flag.ref);let make=|| value;make();make();true}",
    );
    let analysis = luna::resources::analyze_with_protocols(
        &package,
        "run",
        "Duplicate",
        Some("Retire"),
        1_000_000,
    )
    .unwrap();
    let dump = analysis.to_string();
    assert!(
        dump.contains("(captured-pointer-guard-exits (captures (slot"),
        "{dump}"
    );
    assert!(dump.contains("(condition (fields) (variant 0))"), "{dump}");
}

#[test]
fn nominal_default_copy_requires_compile_time_protocol_evidence() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        enum Choice {first(*bool),second(*bool)}";
    let seed = support::compile(prefix);
    let protocols = elaboration::program::Protocols {
        copy: Some(
            seed.traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            seed.traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    for copy in [false, true] {
        let implementation = if copy {
            "impl Duplicate for Choice {}"
        } else {
            ""
        };
        let source = format!(
            "{prefix} {implementation}
            fn supply(value:Choice,.copy:Choice=value)->Choice {{copy}}"
        );
        let compiled = support::compile_with_protocols(&source, &protocols);
        if copy {
            compiled.unwrap();
        } else {
            assert!(
                compiled
                    .unwrap_err()
                    .contains("default expression requires P3")
            );
        }
    }
}

#[test]
fn aggregate_pointer_reads_require_live_copy_storage() {
    for (copy, expired) in [(true, false), (true, true), (false, false)] {
        let implementation = if copy {
            "impl Duplicate for Link {}"
        } else {
            ""
        };
        let storage = if expired {
            "let pointer=do {let link=Link {target:flag.ref};link.ref};"
        } else {
            "let link=Link {target:flag.ref};let pointer=link.ref;"
        };
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}} {implementation}
            fn run()->bool {{let flag=true;{storage}let copied=pointer.*;copied.target.*}}"
        );
        let package = support::compile(&source);
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        );
        if copy && !expired {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert!(result.is_err(), "copy={copy} expired={expired}");
        }
    }
}

#[test]
fn aggregate_pointer_load_preserves_callable_fields() {
    for candidate in ["read", "if flag {read} else {invert}"] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            impl Duplicate for fn(*bool)->bool {{}}
            struct Mixed {{target:*bool,action:fn(*bool)->bool}}
            impl Duplicate for Mixed {{}}
            fn read(p:*bool)->bool {{p.*}}
            fn invert(p:*bool)->bool {{not p.*}}
            fn load(flag:bool,p:*bool)->Mixed {{
                let mixed=Mixed {{target:p,action:{candidate}}};mixed.ref.*
            }}
            fn apply(flag:bool,p:*bool)->bool {{let copied=load(flag,p);copied.action(copied.target)}}
            fn run()->bool {{let flag=true;apply(true,flag.ref)}}"
        );
        let package = support::compile(&source);
        let loader = package
            .functions
            .iter()
            .find(|function| function.name == "load")
            .unwrap();
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            loader.declaration,
            &[],
            &mut 1_000_000,
        )
        .unwrap();
        let apply = package
            .functions
            .iter()
            .find(|function| function.name == "apply")
            .unwrap();
        let applied = elaboration::verification::memory::infer_in_package(
            &package,
            apply.declaration,
            &[],
            &mut 1_000_000,
        )
        .unwrap_or_else(|error| panic!("{candidate}: {error}"));
        assert!(
            applied
                .accesses
                .iter()
                .any(|access| access.target.parameter == 1 && access.read_value && !access.write),
            "{applied}"
        );
        let expected = if candidate == "read" { 1 } else { 2 };
        assert!(
            matches!(summary.code_fields.get(&vec![1]),
            Some(elaboration::verification::memory::CodeResult::Known(values)) if values.len() == expected),
            "{candidate}: {summary}"
        );

        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000,
            )
            .unwrap_or_else(|error| panic!("{candidate}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn captured_aggregate_load_checks_copy_and_pointee_lifetime() {
    for (copy, expired) in [(true, false), (false, false), (true, true)] {
        let implementation = if copy {
            "impl Duplicate for Link {}"
        } else {
            ""
        };
        let target = if expired {
            "do {let local=true;local.ref}"
        } else {
            "flag.ref"
        };
        let package = support::compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}}{implementation}
            fn run()->bool {{let flag=true;let link=Link {{target:{target}}};
                let load=||link.ref.*;let copied=load();copied.target.*}}"
        ));
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        );
        if copy && !expired {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert!(result.is_err(), "copy={copy} expired={expired}");
        }
    }
}

#[test]
fn aggregate_callable_load_joins_every_storage_candidate() {
    use elaboration::verification::memory::{self, CodeResult};
    for unknown in [false, true] {
        let second = if unknown { "action" } else { "invert" };
        let package = support::compile(&format!(
            "struct Holder {{action:fn(*bool)->bool}}
            fn read(p:*bool)->bool {{p.*}}fn invert(p:*bool)->bool {{not p.*}}
            fn load(flag:bool,action:fn(*bool)->bool)->Holder {{
                let first=Holder {{action:read}};let second=Holder {{action:{second}}};
                let pointer=if flag {{first.ref}} else {{second.ref}};pointer.*
            }}"
        ));
        let loader = package
            .functions
            .iter()
            .find(|function| function.name == "load")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, loader.declaration, &[], &mut 1_000_000).unwrap();
        let code = summary.code_fields.get(&vec![0]);
        if unknown {
            assert!(matches!(code, Some(CodeResult::Unknown)), "{summary}");
        } else {
            assert!(
                matches!(code,Some(CodeResult::Known(values)) if values.len()==2),
                "{summary}"
            );
        }
    }
}

#[test]
fn scalar_callable_load_keeps_named_return_summary() {
    use elaboration::verification::memory::{self, CallableResult, CodeResult};
    let package = support::compile(
        "fn read(p:*bool)->bool {p.*}
        fn load()->fn(*bool)->bool {let action=read;action.ref.*}",
    );
    let loader = package
        .functions
        .iter()
        .find(|function| function.name == "load")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, loader.declaration, &[], &mut 1_000_000).unwrap();
    assert!(
        matches!(summary.callable_result, CallableResult::Known(_)),
        "{summary}"
    );
    assert!(
        matches!(summary.code_fields.get(&vec![]),Some(CodeResult::Known(values)) if values.len()==1),
        "{summary}"
    );
}

#[test]
fn loaded_callable_snapshots_follow_read_time_and_field_path() {
    use elaboration::verification::memory::{self, CodeResult, PreparedCode};
    for (body, parameter, action, expected) in [
        (
            "var original=Mixed {target:p,action:read};let copied=original.ref.*;original=Mixed {target:q,action:invert};copied",
            0,
            "read",
            true,
        ),
        (
            "var original=Mixed {target:p,action:read};original=Mixed {target:q,action:invert};original.ref.*",
            1,
            "invert",
            true,
        ),
        (
            "var original=Mixed {target:p,action:read};let copied=original.ref.*;original.action=invert;copied",
            0,
            "read",
            true,
        ),
        (
            "var original=Mixed {target:p,action:read};original.action=invert;original.ref.*",
            0,
            "invert",
            false,
        ),
        (
            "var holder=Holder {inner:Mixed {target:p,action:read}};let copied=holder.inner.ref.*;holder.inner=Mixed {target:q,action:invert};copied",
            0,
            "read",
            true,
        ),
        (
            "var holder=Holder {inner:Mixed {target:p,action:read}};holder.inner=Mixed {target:q,action:read};holder.inner.ref.*",
            1,
            "read",
            false,
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            impl Duplicate for fn(*bool)->bool {{}}
            struct Mixed {{target:*bool,action:fn(*bool)->bool}}impl Duplicate for Mixed {{}}
            struct Holder {{inner:Mixed}}impl Duplicate for Holder {{}}
            fn read(p:*bool)->bool {{p.*}}fn invert(p:*bool)->bool {{not p.*}}
            fn load(p:*bool,q:*bool)->Mixed {{{body}}}
            fn apply(p:*bool,q:*bool)->bool {{let copied=load(p,q);copied.action(copied.target)}}"
        );
        let package = support::compile(&format!(
            "{prefix}fn run()->bool {{let p=true;let q=false;apply(p.ref,q.ref)}}"
        ));
        let loader = package
            .functions
            .iter()
            .find(|function| function.name == "load")
            .unwrap();
        let declaration = package
            .functions
            .iter()
            .find(|function| function.name == action)
            .unwrap()
            .declaration;
        let summary =
            memory::infer_in_package(&package, loader.declaration, &[], &mut 1_000_000).unwrap();
        assert!(
            matches!(summary.code_fields.get(&vec![1]),Some(CodeResult::Known(values))
            if matches!(values.as_slice(),[PreparedCode::Named(key)] if key.declaration==declaration)),
            "{body}: {summary}"
        );
        let targets = summary
            .pointer_fields
            .get(&vec![0])
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(targets.len(), 1, "{body}: {summary}");
        assert_eq!(
            targets.iter().next().unwrap().parameter,
            parameter,
            "{body}: {summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            Value::Bool(expected)
        );
        let expired = if parameter == 0 {
            "apply(expired,q.ref)"
        } else {
            "apply(p.ref,expired)"
        };
        let package = support::compile(&format!(
            "{prefix}fn run()->bool {{let p=true;let q=false;let expired=do {{let local=true;local.ref}};{expired}}}"
        ));
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000
            )
            .is_err(),
            "{body}"
        );
    }
}

#[test]
fn loaded_callback_parameters_preserve_write_contracts() {
    use elaboration::verification::memory;
    for body in [
        "let original=Mixed {target:p,action:callback};original.ref.*",
        "let original=Mixed {target:p,action:callback};let copy=original.ref.*;copy",
        "let original=Mixed {target:p,action:callback};let load=||original.ref.*;load()",
    ] {
        for callback in [
            "write",
            "|q|do {q.* = true;q.*}",
            "if p.* {write} else {|q|do {q.* = true;q.*}}",
        ] {
            let prefix = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            impl Duplicate for fn(*bool)->bool {{}}
            struct Mixed {{target:*bool,action:fn(*bool)->bool}}impl Duplicate for Mixed {{}}
            fn write(p:*bool)->bool {{p.* = true;p.*}}
            fn load(callback:fn(*bool)->bool,p:*bool)->Mixed {{{body}}}
            fn relay(callback:fn(*bool)->bool,p:*bool)->Mixed {{load(callback,p)}}
            fn apply(p:*bool)->bool {{let copied=relay({callback},p);copied.action(copied.target)}}"
        );
            let package = support::compile(&format!(
                "{prefix}fn run()->bool {{var flag=false;let result=apply(flag.ref);result and flag}}"
            ));
            let apply = package
                .functions
                .iter()
                .find(|function| function.name == "apply")
                .unwrap();
            let summary =
                memory::infer_in_package(&package, apply.declaration, &[], &mut 1_000_000)
                    .unwrap_or_else(|error| panic!("{body}: {error}"));
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|access| access.target.parameter == 0 && access.write),
                "{body}: {summary}"
            );
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .unwrap_or_else(|error| panic!("{body}: {error}")),
                Value::Bool(true)
            );
            let package = support::compile(&format!(
                "{prefix}fn run()->bool {{let p=do {{let local=false;local.ref}};apply(p)}}"
            ));
            assert!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .is_err(),
                "{body}"
            );
        }
    }
}

#[test]
fn indirect_aggregate_load_exports_pointer_field_origins() {
    use elaboration::verification::memory;
    for (ty, value, body) in [
        ("Link", "Link {target:target}", "p.*"),
        (
            "Holder",
            "Holder {inner:Link {target:target}}",
            "let copied=p.*;copied.inner",
        ),
        (
            "Wrapped",
            "Link {target:target}.as(Wrapped)",
            "let copied=p.*;copied.as(Link)",
        ),
        (
            "Tuple<Link,bool>",
            "(Link {target:target},false)",
            "let (copied,_)=p.*;copied",
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}}impl Duplicate for Link {{}}
            struct Holder {{inner:Link}}impl Duplicate for Holder {{}}
            newtype Wrapped=Link;impl Duplicate for Wrapped {{}}
            impl Duplicate for Tuple<Link,bool> {{}}
            fn load(p:*{ty})->Link {{{body}}}
            fn relay(p:*{ty})->Link {{load(p)}}"
        );
        for (target, storage, valid) in [
            (
                "flag.ref",
                format!("let value={value};let pointer=value.ref;"),
                true,
            ),
            (
                "do {let local=true;local.ref}",
                format!("let value={value};let pointer=value.ref;"),
                false,
            ),
            (
                "flag.ref",
                format!("let pointer=do {{let value={value};value.ref}};"),
                false,
            ),
        ] {
            let package = support::compile_with_tuple(&format!(
                "{prefix}fn run()->bool {{let flag=true;let target={target};{storage}let copied=relay(pointer);copied.target.*}}"
            ));
            let loader = package
                .functions
                .iter()
                .find(|function| function.name == "load")
                .unwrap();
            let summary =
                memory::infer_in_package(&package, loader.declaration, &[], &mut 1_000_000)
                    .unwrap();
            let targets = summary
                .pointer_fields
                .get(&vec![0])
                .unwrap()
                .as_ref()
                .unwrap();
            assert!(
                targets
                    .iter()
                    .all(|target| target.parameter == 0 && target.through.len() == 1),
                "{ty}: {summary}"
            );
            assert_eq!(targets.len(), 1, "{ty}: {summary}");
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000,
            );
            if valid {
                assert_eq!(
                    result.unwrap_or_else(|error| panic!("{ty}: {error}")),
                    Value::Bool(true)
                );
            } else {
                assert!(result.is_err(), "{ty}: {target}: {storage}");
            }
        }
    }
}

#[test]
fn indirect_aggregate_load_keeps_each_slot_and_snapshot() {
    use elaboration::verification::memory;
    for (parameters, body, prepare, call, depth, alternatives) in [
        (
            "p:**Link",
            "p.*.*",
            "let slot=original.ref;",
            "load(slot.ref)",
            2,
            1,
        ),
        (
            "p:***Link",
            "p.*.*.*",
            "let slot=original.ref;let outer=slot.ref;",
            "load(outer.ref)",
            3,
            1,
        ),
        (
            "flag:bool,p:*Link,q:*Link",
            "let pointer=if flag {p} else {q};pointer.*",
            "let other=Link {target:second.ref};",
            "load(true,original.ref,other.ref)",
            1,
            2,
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}}impl Duplicate for Link {{}}
            fn load({parameters})->Link {{{body}}}"
        );
        let package = support::compile(&format!(
            "{prefix}fn run()->bool {{let first=true;let second=false;var original=Link {{target:first.ref}};{prepare}let copied={call};original=Link {{target:second.ref}};copied.target.*}}"
        ));
        let loader = package
            .functions
            .iter()
            .find(|function| function.name == "load")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, loader.declaration, &[], &mut 1_000_000).unwrap();
        let targets = summary
            .pointer_fields
            .get(&vec![0])
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(targets.len(), alternatives, "{summary}");
        assert!(
            targets.iter().all(|target| target.through.len() == depth),
            "{summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000
            )
            .unwrap_or_else(|error| panic!("{parameters}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn indirect_aggregate_load_rejects_expired_intermediate_slots() {
    for (ty, body, prepare, call) in [
        (
            "**Link",
            "p.*.*",
            "let slot=do {let local=Link {target:flag.ref};local.ref};",
            "load(slot.ref)",
        ),
        (
            "**Link",
            "p.*.*",
            "let pointer=do {let slot=link.ref;slot.ref};",
            "load(pointer)",
        ),
        (
            "***Link",
            "p.*.*.*",
            "let outer=do {let slot=link.ref;slot.ref};",
            "load(outer.ref)",
        ),
        (
            "***Link",
            "p.*.*.*",
            "let pointer=do {let slot=link.ref;let outer=slot.ref;outer.ref};",
            "load(pointer)",
        ),
    ] {
        let package=support::compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
            struct Link {{target:*bool}}impl Duplicate for Link {{}}
            fn load(p:{ty})->Link {{{body}}}
            fn run()->bool {{let flag=true;let link=Link {{target:flag.ref}};{prepare}let copied={call};copied.target.*}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap_err();
        assert!(error.contains("unavailable"), "{prepare}: {error}");
    }
}

#[test]
fn callable_subtree_reads_exclude_siblings_and_other_roots() {
    use elaboration::verification::memory::{self, CodeResult, PreparedCode};
    for suffix in ["", ".ref.*"] {
        for (selection, action, parameter) in
            [("holder.left", "read", 0), ("holder.right", "invert", 1)]
        {
            let noise = (0..32)
                .map(|index| format!("let noise{index}=Mixed {{target:q,action:invert}};"))
                .collect::<String>();
            let package=support::compile(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                impl Duplicate for bool {{}}impl Duplicate for *T where T {{}}
                impl Duplicate for fn(*bool)->bool {{}}
                struct Mixed {{target:*bool,action:fn(*bool)->bool}}impl Duplicate for Mixed {{}}
                struct Holder {{left:Mixed,right:Mixed}}impl Duplicate for Holder {{}}
                fn read(p:*bool)->bool {{p.*}}fn invert(p:*bool)->bool {{not p.*}}
                fn load(p:*bool,q:*bool)->Mixed {{
                    {noise}
                    let holder=Holder {{left:Mixed {{target:p,action:read}},right:Mixed {{target:q,action:invert}}}};
                    let trailing=Mixed {{target:q,action:invert}};{selection}{suffix}
                }}
                fn run()->bool {{let p=true;let q=false;let result=load(p.ref,q.ref);result.action(result.target)}}"
            ));
            let loader = package
                .functions
                .iter()
                .find(|function| function.name == "load")
                .unwrap();
            let declaration = package
                .functions
                .iter()
                .find(|function| function.name == action)
                .unwrap()
                .declaration;
            let summary =
                memory::infer_in_package(&package, loader.declaration, &[], &mut 1_000_000)
                    .unwrap();
            assert!(
                matches!(summary.code_fields.get(&vec![1]),Some(CodeResult::Known(values))
                if matches!(values.as_slice(),[PreparedCode::Named(key)] if key.declaration==declaration)),
                "{selection}{suffix}: {summary}"
            );
            let targets = summary
                .pointer_fields
                .get(&vec![0])
                .unwrap()
                .as_ref()
                .unwrap();
            assert_eq!(targets.len(), 1, "{summary}");
            assert_eq!(
                targets.iter().next().unwrap().parameter,
                parameter,
                "{summary}"
            );
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn conditional_storage_projection_preserves_nested_tag_requirements() {
    use elaboration::verification::memory::project_owned_storage;
    let package = support::compile(
        "typealias u32=.u32'builtin;
        enum Inner {flag(*bool),count(*u32)}enum Outer {none,payload(Inner)}
        struct Root {inner:Outer}",
    );
    let root = nir::Term::Nominal {
        declaration: package
            .structs
            .iter()
            .find(|item| item.name == "Root")
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    let mut projections = vec![];
    for tag in [0, 1] {
        let projection = project_owned_storage(
            Some(&package),
            &root,
            &[0, usize::MAX, 1, 0, usize::MAX, tag, 0],
            &mut 10_000,
        )
        .unwrap();
        assert_eq!(
            projection
                .fields
                .iter()
                .map(|(_, index)| *index)
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert_eq!(projection.guards, vec![(vec![0], 1), (vec![0, 0], tag)]);
        let inner = package
            .enums
            .iter()
            .find(|item| item.name == "Inner")
            .unwrap();
        assert_eq!(projection.ty, inner.variants[tag].fields[0]);
        let dump = projection.to_string();
        assert!(dump.contains("(guard (fields 0) (variant 1))"), "{dump}");
        assert!(
            dump.contains(&format!("(guard (fields 0 0) (variant {tag}))")),
            "{dump}"
        );
        assert_eq!(
            dump.matches('(').count(),
            dump.matches(')').count(),
            "{dump}"
        );
        let path = [0, usize::MAX, 1, 0, usize::MAX, tag, 0];
        for split in [0, 1, 4, 7] {
            let prefix =
                project_owned_storage(Some(&package), &root, &path[..split], &mut 10_000).unwrap();
            let composed = prefix
                .then(Some(&package), &path[split..], &mut 10_000)
                .unwrap();
            assert_eq!(composed, projection, "split={split}, tag={tag}");
            assert_eq!(composed.to_string(), projection.to_string());
        }
        projections.push(projection);
    }
    let prefix = project_owned_storage(Some(&package), &root, &[0], &mut 10_000).unwrap();
    let saved = prefix.clone();
    assert!(
        prefix
            .then(
                Some(&package),
                &[usize::MAX, 1, 0, usize::MAX, 9, 0],
                &mut 10_000
            )
            .is_err()
    );
    assert_eq!(prefix, saved);
    assert!(
        prefix
            .then(Some(&package), &[usize::MAX, 1, 0], &mut 0)
            .is_err()
    );
    assert_eq!(prefix, saved);
    let depth_error = prefix
        .then(Some(&package), &[0; 128], &mut 10_000)
        .unwrap_err();
    assert!(depth_error.contains("depth exhausted"), "{depth_error}");
    assert_eq!(prefix, saved);
    assert_eq!(
        prefix
            .then(
                Some(&package),
                &[usize::MAX, 1, 0, usize::MAX, 0, 0],
                &mut 10_000
            )
            .unwrap(),
        projections[0]
    );
    assert_eq!(projections[0].fields, projections[1].fields);
    assert_ne!(projections[0].guards, projections[1].guards);
    assert_ne!(projections[0].ty, projections[1].ty);
    let wrapped = nir::Term::Record(vec![
        ("padding".into(), nir::Term::Primitive(nir::Primitive::Bool)),
        ("value".into(), root.clone()),
    ]);
    let envelope = project_owned_storage(Some(&package), &wrapped, &[1], &mut 10_000).unwrap();
    let composed = envelope
        .then(
            Some(&package),
            &[0, usize::MAX, 1, 0, usize::MAX, 0, 0],
            &mut 10_000,
        )
        .unwrap();
    let complete = project_owned_storage(
        Some(&package),
        &wrapped,
        &[1, 0, usize::MAX, 1, 0, usize::MAX, 0, 0],
        &mut 10_000,
    )
    .unwrap();
    assert_eq!(composed, complete);
    assert_eq!(composed.guards, vec![(vec![1, 0], 1), (vec![1, 0, 0], 0)]);
    let before = composed.clone();
    assert!(composed.then(Some(&package), &[0], &mut 10_000).is_err());
    assert_eq!(composed, before);

    for path in [
        vec![0, 0],
        vec![0, usize::MAX],
        vec![0, usize::MAX, 2, 0],
        vec![0, usize::MAX, 0, 0],
        vec![0, usize::MAX, 1, 3],
    ] {
        assert!(
            project_owned_storage(Some(&package), &root, &path, &mut 10_000).is_err(),
            "{path:?}"
        );
    }
    assert!(project_owned_storage(Some(&package), &root, &[0], &mut 0).is_err());
    let plain = nir::Term::Record(vec![(
        "flag".into(),
        nir::Term::Primitive(nir::Primitive::Bool),
    )]);
    let projected = project_owned_storage(None, &plain, &[0], &mut 100).unwrap();
    assert!(projected.guards.is_empty());
    assert_eq!(projected.ty, nir::Term::Primitive(nir::Primitive::Bool));
    assert!(project_owned_storage(None, &root, &[0], &mut 10_000).is_err());
}

#[test]
fn conditional_storage_traversal_separates_loaded_roots() {
    use elaboration::verification::memory::StorageTraversal;
    let package = support::compile(
        "enum Inner {flag(*bool)}enum Outer {none,payload(*Inner)}
        struct Root {padding:bool,outer:Outer}enum Node {next(*Node)}",
    );
    let root = nir::Term::Nominal {
        declaration: package
            .structs
            .iter()
            .find(|item| item.name == "Root")
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    let mut traversal =
        StorageTraversal::new(Some(&package), &root, &[1, usize::MAX, 1, 0], &mut 10_000).unwrap();
    let mut segmented = StorageTraversal::new(Some(&package), &root, &[1], &mut 10_000).unwrap();
    segmented
        .then(Some(&package), &[usize::MAX, 1, 0], &mut 10_000)
        .unwrap();
    assert_eq!(segmented, traversal);
    let saved = traversal.clone();
    assert!(
        traversal
            .load(Some(&package), &[usize::MAX, 9, 0], &mut 10_000)
            .is_err()
    );
    assert_eq!(traversal, saved);
    assert!(traversal.load(Some(&package), &[], &mut 0).is_err());
    assert_eq!(traversal, saved);
    traversal
        .load(Some(&package), &[usize::MAX, 0, 0], &mut 10_000)
        .unwrap();
    segmented.load(Some(&package), &[], &mut 10_000).unwrap();
    segmented
        .then(Some(&package), &[usize::MAX, 0, 0], &mut 10_000)
        .unwrap();
    assert_eq!(segmented, traversal);
    assert_eq!(traversal.loads[0].guards, vec![(vec![1], 1)]);
    assert_eq!(traversal.target.guards, vec![(vec![], 0)]);
    traversal.load(Some(&package), &[], &mut 10_000).unwrap();
    assert_eq!(traversal.loads.len(), 2);
    assert_eq!(traversal.loads[1].guards, vec![(vec![], 0)]);
    assert!(traversal.target.guards.is_empty());
    assert!(traversal.target.fields.is_empty());
    assert_eq!(
        traversal.target.ty,
        nir::Term::Primitive(nir::Primitive::Bool)
    );
    let dump = traversal.to_string();
    assert_eq!(
        dump.matches("(load (storage-projection").count(),
        2,
        "{dump}"
    );
    assert_eq!(
        dump.matches('(').count(),
        dump.matches(')').count(),
        "{dump}"
    );
    let saved = traversal.clone();
    assert!(
        traversal
            .load(Some(&package), &[], &mut 10_000)
            .unwrap_err()
            .contains("not a pointer")
    );
    assert_eq!(traversal, saved);
    let node = nir::Term::Nominal {
        declaration: package
            .enums
            .iter()
            .find(|item| item.name == "Node")
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    let mut recursive =
        StorageTraversal::new(Some(&package), &node, &[usize::MAX, 0, 0], &mut 10_000).unwrap();
    for _ in 0..31 {
        recursive
            .load(Some(&package), &[usize::MAX, 0, 0], &mut 10_000)
            .unwrap();
    }
    let saved = recursive.clone();
    assert!(
        recursive
            .load(Some(&package), &[usize::MAX, 0, 0], &mut 10_000)
            .unwrap_err()
            .contains("depth exhausted")
    );
    assert_eq!(recursive, saved);
}

#[test]
fn storage_guard_activity_uses_conditioned_domains_and_short_circuits() {
    use elaboration::verification::memory::{
        GuardActivity as A, project_owned_storage, storage_guard_activity,
    };
    let package = support::compile(
        "enum Inner {flag(*bool),none}enum Outer {none,branch(Inner)}struct Root {entry:Outer}",
    );
    let root = nir::Term::Nominal {
        declaration: package.structs[0].declaration,
        arguments: vec![],
    };
    let projection = project_owned_storage(
        Some(&package),
        &root,
        &[0, usize::MAX, 1, 0, usize::MAX, 0, 0],
        &mut 10_000,
    )
    .unwrap();
    for (outer, inner, expected) in [
        (A::Active, A::Active, A::Active),
        (A::Inactive, A::Active, A::Inactive),
        (A::Conditional, A::Active, A::Conditional),
        (A::Active, A::Inactive, A::Inactive),
        (A::Conditional, A::Inactive, A::Inactive),
        (A::Active, A::Conditional, A::Conditional),
        (A::Conditional, A::Conditional, A::Conditional),
    ] {
        let mut calls = 0;
        let result = projection
            .activity(&mut 10_000, |path, tag| {
                calls += 1;
                if calls == 1 {
                    assert_eq!(path, &[0]);
                    assert_eq!(tag, 1);
                    outer
                } else {
                    assert_eq!(calls, 2);
                    assert_eq!(path, &[0, usize::MAX, 1, 0]);
                    assert_eq!(tag, 0);
                    inner
                }
            })
            .unwrap();
        assert_eq!(result, expected);
        assert_eq!(calls, if outer == A::Inactive { 1 } else { 2 });
    }
    assert!(
        projection
            .activity(&mut 0, |_, _| panic!("query after budget exhaustion"))
            .is_err()
    );
    assert_eq!(
        storage_guard_activity(&[], &mut 0, |_, _| panic!("empty guard query")).unwrap(),
        A::Active
    );
    for guards in [
        vec![(vec![1], 0), (vec![0, 0], 1)],
        vec![(vec![0], 0), (vec![0], 1)],
    ] {
        assert!(storage_guard_activity(&guards, &mut 10_000, |_, _| A::Active).is_err());
    }
    assert_eq!(A::Conditional.to_string(), "(guard-activity conditional)");
}

#[test]
fn effect_forwarded_absent_payloads_do_not_create_pointer_slots() {
    let package = support::compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        enum Envelope {none,some(*bool)}impl Duplicate for Envelope {}
        effect choose(flag:bool,left:Envelope,right:Envelope)->Envelope;
        fn run()->bool {let result=choose(true,Envelope.none,Envelope.none)# {
            choose(flag,left,right)=>if flag {left} else {right},
        }; result match {.none=>true,.some(_)=>false,}}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn conditional_storage_transport_validates_owners_tags_and_relative_roots() {
    use elaboration::verification::memory::{StorageTraversal, project_owned_storage};
    let package = support::compile(
        "typealias u32=.u32'builtin; enum Inner {flag(*bool),count(*u32)}
         enum Outer {none,payload(Inner)} struct Root {padding:bool,outer:Outer}",
    );
    let root = nir::Term::Nominal {
        declaration: package
            .structs
            .iter()
            .find(|item| item.name == "Root")
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    for tag in 0..2 {
        let expected = project_owned_storage(
            Some(&package),
            &root,
            &[1, usize::MAX, 1, 0, usize::MAX, tag, 0],
            &mut 10_000,
        )
        .unwrap();
        let start = StorageTraversal::new(Some(&package), &root, &[], &mut 10_000).unwrap();
        let mut actual = start.clone();
        actual
            .then_guarded(
                Some(&package),
                &expected.fields,
                &expected.guards,
                &mut 10_000,
            )
            .unwrap();
        assert_eq!(actual.target, expected);
        let mut segmented =
            StorageTraversal::new(Some(&package), &root, &[1], &mut 10_000).unwrap();
        let suffix = project_owned_storage(
            Some(&package),
            &segmented.target.ty,
            &[usize::MAX, 1, 0, usize::MAX, tag, 0],
            &mut 10_000,
        )
        .unwrap();
        segmented
            .then_guarded(Some(&package), &suffix.fields, &suffix.guards, &mut 10_000)
            .unwrap();
        assert_eq!(segmented, actual);
        let saved = actual.target.clone();
        actual.load(Some(&package), &[], &mut 10_000).unwrap();
        assert_eq!(actual.loads, vec![saved]);
        assert!(actual.target.guards.is_empty());
        assert_eq!(
            actual.target.ty,
            nir::Term::Primitive(if tag == 0 {
                nir::Primitive::Bool
            } else {
                nir::Primitive::U32
            })
        );

        let mut bad_owner = expected.fields.clone();
        bad_owner[0].0 = nir::program::FieldOwner::Anonymous;
        let mut bad_tag = expected.guards.clone();
        bad_tag[1].1 = 9;
        let mut bad_prefix = expected.guards.clone();
        bad_prefix[1].0[0] = 0;
        let mut duplicate = expected.guards.clone();
        duplicate.push(duplicate[1].clone());
        let mut reversed = expected.guards.clone();
        reversed.reverse();
        let mut trailing = expected.guards.clone();
        trailing.push((vec![1, 0, 0], 0));
        for (fields, guards) in [
            (bad_owner, expected.guards.clone()),
            (expected.fields.clone(), vec![]),
            (expected.fields.clone(), bad_tag),
            (expected.fields.clone(), bad_prefix),
            (expected.fields.clone(), duplicate),
            (expected.fields.clone(), reversed),
            (expected.fields.clone(), trailing),
        ] {
            let mut rejected = start.clone();
            assert!(
                rejected
                    .then_guarded(Some(&package), &fields, &guards, &mut 10_000)
                    .is_err()
            );
            assert_eq!(rejected, start);
            rejected
                .then_guarded(
                    Some(&package),
                    &expected.fields,
                    &expected.guards,
                    &mut 10_000,
                )
                .unwrap();
            assert_eq!(rejected.target, expected);
        }
        let mut rejected = start.clone();
        assert!(
            rejected
                .then_guarded(Some(&package), &expected.fields, &expected.guards, &mut 0)
                .is_err()
        );
        assert_eq!(rejected, start);
    }
}

#[test]
fn indirect_enum_load_preserves_conditional_pointer_origins() {
    use elaboration::verification::memory;
    for (ty, storage, pointer, body) in [
        ("Choice", "value", "stored.ref", "p.*"),
        ("*Choice", "value.ref", "stored.ref", "p.*.*"),
        (
            "Holder",
            "Holder {padding:false,value:value}",
            "stored.ref",
            "let copied=p.*;copied.value",
        ),
        (
            "Envelope",
            ".slot(value)",
            "stored.ref",
            "let copied=p.*;copied match {.none=>.none,.slot(inner)=>inner}",
        ),
    ] {
        let prefix = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            typealias u32=.u32'builtin;
            impl Duplicate for bool {{}}impl Duplicate for u32 {{}}impl Duplicate for *T where T {{}}
            enum Choice {{none,flag(*bool),count(*u32)}}impl Duplicate for Choice {{}}
            struct Holder {{padding:bool,value:Choice}}impl Duplicate for Holder {{}}
            enum Envelope {{none,slot(Choice)}}impl Duplicate for Envelope {{}}
            fn load(p:*{ty})->Choice {{{body}}}
            fn relay(p:*{ty})->Choice {{load(p)}}
            fn choose(flag:bool,left:*bool,right:*u32)->Choice {{if flag {{.flag(left)}} else {{.count(right)}}}}
            fn examine(value:Choice)->bool {{value match {{.none=>true,.flag(p)=>p.*,.count(p)=>p.* == 7}}}}"
        );
        for value in [
            "Choice.none",
            "Choice.flag(flag.ref)",
            "Choice.count(count.ref)",
            "choose(flag,flag.ref,count.ref)",
            "choose(not flag,flag.ref,count.ref)",
        ] {
            let source = format!("{prefix}fn run()->bool {{let flag=true;let count:u32=7;
                let value={value};let stored:{ty} = {storage};let copied=relay({pointer});examine(copied)}}");
            let package = support::compile(&source);
            let summary = memory::infer_in_package(
                &package,
                support::id(&package, "relay"),
                &[],
                &mut 1_000_000,
            )
            .unwrap();
            assert!(
                summary
                    .pointer_fields
                    .values()
                    .any(|origins| origins.as_ref().is_some_and(|origins| origins
                        .iter()
                        .any(|origin| !origin.conditions.is_empty()))),
                "{ty}: {summary}"
            );
            let dump = summary.to_string();
            assert!(dump.contains("(conditions (stage"), "{dump}");
            assert_eq!(dump.matches('(').count(), dump.matches(')').count());
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .unwrap_or_else(|error| panic!("{ty}, {value}: {error}")),
                Value::Bool(true)
            );
        }
        for value in [
            "Choice.flag(do {let local=true;local.ref})",
            "Choice.count(do {let local:u32=7;local.ref})",
        ] {
            let source = format!(
                "{prefix}fn run()->bool {{let value={value};let stored:{ty} = {storage};examine(relay({pointer}))}}"
            );
            let package = support::compile(&source);
            assert!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000
                )
                .is_err(),
                "{ty}: expired {value}"
            );
        }
    }
}

#[test]
fn indirect_enum_load_rejects_expired_owners_and_keeps_read_snapshot() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        enum Choice {none,flag(*bool)}impl Duplicate for Choice {}
        fn load(p:*Choice)->Choice {p.*}
        fn examine(value:Choice)->bool {value match {.none=>true,.flag(p)=>p.*}}";
    for value in ["Choice.none", "Choice.flag(flag.ref)"] {
        let package = support::compile(&format!(
            "{prefix}fn run()->bool {{let flag=true;
            let p=do {{let value={value};value.ref}};examine(load(p))}}"
        ));
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000
            )
            .is_err()
        );
    }
    let package = support::compile(&format!("{prefix}fn run()->bool {{let flag=true;
        var value=Choice.flag(flag.ref);let copied=load(value.ref);value=Choice.none;examine(copied)}}"));
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_enum_load_can_consume_the_copy_inside_the_callee() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias u32=.u32'builtin;
        impl Duplicate for bool {}impl Duplicate for u32 {}impl Duplicate for *T where T {}
        enum Choice {none,flag(*bool),count(*u32)}impl Duplicate for Choice {}
        fn read(p:*Choice)->bool {let copied=p.*;
            copied match {.none=>true,.flag(p)=>p.*,.count(p)=>p.* == 7}}
        fn choose(flag:bool,a:*bool,b:*u32)->Choice {if flag {.flag(a)} else {.count(b)}}";
    for value in [
        "Choice.none",
        "Choice.flag(flag.ref)",
        "Choice.count(count.ref)",
        "choose(flag,flag.ref,count.ref)",
        "choose(not flag,flag.ref,count.ref)",
    ] {
        let package = support::compile(&format!(
            "{prefix}fn run()->bool {{let flag=true;let count:u32=7;let value={value};read(value.ref)}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000
            )
            .unwrap_or_else(|error| panic!("{value}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn indirect_enum_load_preserves_conditions_on_each_loaded_root() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        enum Choice {none,flag(*bool)}impl Duplicate for Choice {}
        enum Handle {none,ptr(*Choice)}impl Duplicate for Handle {}
        fn load(p:*Handle)->Choice {let handle=p.*;handle match {.none=>.none,.ptr(inner)=>inner.*}}
        fn relay(p:*Handle)->Choice {load(p)}";
    for (handle, valid) in [
        ("Handle.none", true),
        ("Handle.ptr(value.ref)", true),
        (
            "do {let expired=Choice.flag(flag.ref);Handle.ptr(expired.ref)}",
            false,
        ),
    ] {
        let package=support::compile(&format!("{prefix}fn run()->bool {{let flag=true;let value=Choice.flag(flag.ref);
            let handle={handle};let copied=relay(handle.ref);copied match {{.none=>true,.flag(p)=>p.*}}}}"));
        let summary = memory::infer_in_package(
            &package,
            support::id(&package, "relay"),
            &[],
            &mut 1_000_000,
        )
        .unwrap();
        assert!(
            summary
                .pointer_fields
                .values()
                .filter_map(Option::as_ref)
                .flatten()
                .any(|target| target.through.len() == 2
                    && target.conditions.contains_key(&0)
                    && target.conditions.contains_key(&1)),
            "{summary}"
        );
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        );
        if valid {
            assert_eq!(
                result.unwrap_or_else(|error| panic!("{handle}: {error}")),
                Value::Bool(true)
            );
        } else {
            assert!(result.is_err(), "expired intermediate enum accepted");
        }
    }
}

#[test]
fn scalar_passing_request_footprint_is_independent_of_provider_storage() {
    use elaboration::verification::memory;
    let package = support::compile(
        "^attrs.effects.direct_passing effect value()->T where T;
         fn subject()->#value<T> T where T {value<T>()#}",
    );
    for primitive in [
        nir::Primitive::Bool,
        nir::Primitive::U128,
        nir::Primitive::F64,
    ] {
        let arguments = [nir::Term::Primitive(primitive)];
        let infer = |fuel: &mut usize| {
            memory::infer_in_package(&package, support::id(&package, "subject"), &arguments, fuel)
        };
        assert!(infer(&mut 0).is_err());
        let summary = infer(&mut 200000).unwrap();
        assert!(summary.accesses.is_empty(), "{summary}");
        // Empty memory footprint does not replace the separate conservative rejection analysis.
        assert!(summary.may_reject, "{summary}");
        assert!(!summary.may_run_cleanup, "{summary}");
        assert_eq!(summary, infer(&mut 200000).unwrap());
    }
}

#[test]
fn scalar_passing_request_rule_does_not_erase_provider_access_or_rejection() {
    use elaboration::verification::memory;
    let package = support::compile(
        "^attrs.effects.direct_passing effect ready()->bool;
         fn subject()->#ready bool {ready()#}
         fn install(target:*bool)->bool {
             subject()# {ready()=>do {assumes target.*; target.* = false; true},}
         }",
    );
    let summary =
        memory::infer_in_package(&package, support::id(&package, "install"), &[], &mut 200000)
            .unwrap();
    assert!(
        summary.accesses.iter().any(|access| access.read_value),
        "{summary}"
    );
    assert!(
        summary.accesses.iter().any(|access| access.write),
        "{summary}"
    );
    assert!(summary.may_reject, "{summary}");
}

#[test]
fn scalar_passing_request_rule_rejects_direct_pointer_and_forged_abi() {
    use elaboration::verification::memory;
    for (attribute, result) in [("", "bool"), ("^attrs.effects.direct_passing", "*bool")] {
        let package = support::compile(&format!(
            "{attribute} effect ready()->{result}; fn subject()->#ready {result} {{ready()#}}"
        ));
        let error =
            memory::infer_in_package(&package, support::id(&package, "subject"), &[], &mut 200000)
                .unwrap_err();
        assert!(
            error.contains("installed handler memory premise"),
            "{error}"
        );
    }
    let mut package = support::compile(
        "^attrs.effects.direct_passing effect ready()->bool; fn subject()->#ready bool {ready()#}",
    );
    let id = support::id(&package, "subject");
    for function in &mut package.functions {
        nir::visit::expression_mut(&mut function.body, &mut 10000, |expr, _| {
            if let nir::program::ExprKind::EffectRequest { abi, .. } = &mut expr.kind {
                *abi = nir::effects::Abi::Direct;
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(
        memory::infer_in_package(&package, id, &[], &mut 200000)
            .unwrap_err()
            .contains("disagrees with its premise")
    );
}

#[test]
fn passing_newtype_memory_summary_inspects_representation_without_copy_assumptions() {
    use elaboration::verification::memory;
    for (payload, accepted) in [("bool", true), ("*bool", false), ("fn()->bool", false)] {
        let package = support::compile(&format!(
            "newtype Wrapper<T> = T;
            ^attrs.effects.direct_passing effect value()->Wrapper<{payload}>;
            fn subject()->#value Wrapper<{payload}> {{value()#}}"
        ));
        let result =
            memory::infer_in_package(&package, support::id(&package, "subject"), &[], &mut 200000);
        if accepted {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .contains("installed handler memory premise")
            );
        }
    }
}

#[test]
fn passing_error_result_memory_checks_success_and_all_error_payloads() {
    use elaboration::verification::memory;
    for (result, accepted) in [
        ("!Payload bool", true),
        ("!Borrowed bool", false),
        ("!Payload *bool", false),
        ("![Payload,Borrowed] bool", false),
    ] {
        let package = support::compile(&format!(
            "struct Payload {{flag:bool}} struct Borrowed {{target:*bool}}
            ^attrs.effects.direct_passing effect answer()->{result};
            fn subject()->#answer {result} {{answer()#}}"
        ));
        let result =
            memory::infer_in_package(&package, support::id(&package, "subject"), &[], &mut 200000);
        if accepted {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .contains("installed handler memory premise")
            );
        }
    }
}

#[test]
fn data_error_catches_keep_before_arm_and_after_memory_writes() {
    use elaboration::verification::memory;
    use std::collections::BTreeSet;
    let package = support::compile(
        "typealias Void=.void'builtin;
        fn step(failed:bool)->!bool Void {if failed {return error true;}}
        fn subject(failed:bool,before:*bool,arm:*bool,after:*bool) {
            step(failed)! {
                catch original=>{before.* = true;},
                true=>{arm.* = true;},
                catch original=>{after.* = true;},
            };
        }",
    );
    let id = support::id(&package, "subject");
    assert!(memory::infer_in_package(&package, id, &[], &mut 0).is_err());
    let summary = memory::infer_in_package(&package, id, &[], &mut 200000).unwrap();
    let written: BTreeSet<_> = summary
        .accesses
        .iter()
        .filter(|access| access.write)
        .map(|access| access.target.parameter)
        .collect();
    assert_eq!(written, BTreeSet::from([1, 2, 3]));
}

#[test]
fn handler_whole_capture_replacement_preserves_each_pointer_origin() {
    for replacement in [
        "state=State{target:state.target,next:not state.next};",
        "state=advance(state);",
    ] {
        let source = format!(
            "
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            struct State {{target:*bool,next:bool}} impl Duplicate for State {{}}
            fn advance(state:State)->State {{State{{target:state.target,next:not state.next}}}}
            effect touch()->bool;
            fn twice()->#touch bool {{let first=touch()#;let second=touch()#;not first and second}}
            fn run()->bool {{var flag=false;var state=State{{target:flag.ref,next:false}};
                let result=twice()# {{touch()=>do {{
                    state.target.* = state.next;{replacement}state.target.*
                }},}};
                result and flag and not state.next
            }}
        "
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000
            )
            .unwrap_or_else(|error| panic!("{replacement}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn stationary_handler_exit_does_not_authorize_installer_rebinding() {
    let source = "
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct State {target:*bool} impl Duplicate for State {}
        effect touch()->bool;
        fn run()->bool {
            var first=false;var second=false;var state=State{target:first.ref};
            do {state=State{target:second.ref};touch()#}# {
                touch()=>do {state=State{target:state.target};state.target.*},
            }
        }
    ";
    let package = support::compile(source);
    assert_capture_writeback_rejected(&package);
}

#[test]
fn handler_capture_exit_rejects_changed_or_joined_pointer_fields() {
    for replacement in [
        "state=State{left:state.right,right:state.left};",
        "state=if choose {State{left:state.right,right:state.right}} else {state};",
    ] {
        let source=format!("
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            struct State {{left:*bool,right:*bool}} impl Duplicate for State {{}}
            effect touch(choose:bool)->bool;
            fn run()->bool {{var first=false;var second=true;var state=State{{left:first.ref,right:second.ref}};
                touch(true)# {{touch(choose)=>do {{{replacement}state.left.*}},}}
            }}
        ");
        let package = support::compile(&source);
        assert_capture_writeback_rejected(&package);
    }
}

#[test]
fn handler_capture_restoration_does_not_hide_guard_or_suspension_mutations() {
    let prefix = "
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct State {target:*bool} impl Duplicate for State {}
        effect touch()->bool; effect notify()->bool;
    ";
    for expression in [
        "touch()# {touch() if do {state=State{target:other};false}=>do {state=saved;state.target.*},touch()=>state.target.*,}",
        "(touch()# {touch()=>do {let old=state;state=State{target:other};let ignored=notify()#;state=old;state.target.*},})# {notify()=>state.target.*,}",
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{var first=false;var second=true;var state=State{{target:first.ref}};let saved=state;let other=second.ref;{expression}}}"
        );
        let package = support::compile(&source);
        if expression.contains(" if ") {
            let error = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000,
            )
            .unwrap_err();
            assert!(
                error.contains("observation region changes external state"),
                "{error}"
            );
        } else {
            assert_capture_writeback_rejected(&package);
        }
    }
}

fn assert_capture_writeback_rejected(package: &nir::program::Package) {
    let error =
        luna::execution::interpret_with_protocols(package, "run", "Duplicate", "Retire", 1_000_000)
            .unwrap_err();
    assert!(
        error.contains("writeback contract") || error.contains("pointer slot rebinding"),
        "{error}"
    );
}

#[test]
fn stationary_capture_writes_compose_across_handler_arms() {
    let source="
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct State {target:*bool,next:bool} impl Duplicate for State {}
        effect touch(which:bool)->bool;
        fn twice()->#touch bool {let first=touch(false)#;let second=touch(true)#;not first and second}
        fn run()->bool {
            var flag=false;var state=State{target:flag.ref,next:false};
            let result=twice()# {
                touch(false)=>do {
                    state.target.* = state.next;
                    state=State{target:state.target,next:true};
                    state.target.*
                },
                touch(true)=>do {
                    state.target.* = state.next;
                    state=State{target:state.target,next:false};
                    state.target.*
                },
            };
            result and flag and not state.next
        }
    ";
    let separate_operations = source
        .replace(
            "effect touch(which:bool)->bool;",
            "effect touch(which:bool)->bool; effect start()->bool; effect finish()->bool;",
        )
        .replace(
            "let result=twice()#",
            "let result=do {let first=start()#;let second=finish()#;not first and second}#",
        )
        .replace("touch(false)=>", "start()=>")
        .replace("touch(true)=>", "finish()=>");
    for source in [source, separate_operations.as_str()] {
        let package = support::compile(source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000
            )
            .unwrap(),
            Value::Bool(true),
        );
        let changed = source.replace(
            "state=State{target:state.target,next:false};",
            "var local=false;state=State{target:local.ref,next:false};",
        );
        assert_capture_writeback_rejected(&support::compile(&changed));
    }
}
