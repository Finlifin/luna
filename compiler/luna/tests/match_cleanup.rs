//! Cleanup identities of desugared matches, including both synthetic outcomes.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use support::{calls, compile, id};

#[test]
fn divergent_match_subject_does_not_invent_loop_exits() {
    for arms in [
        "_ if false => break, _ => continue,",
        "_ if do {break;} => continue, _ => break,",
        "_ => break,",
    ] {
        let package = compile(&format!(
            "fn run()->bool {{
                while true {{(do {{return true;}}) match {{{arms}}};}}
            }}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 10_000).unwrap(),
            Value::Bool(true),
            "{arms}"
        );
    }
}

fn execute(source: &str) -> Vec<String> {
    use elaboration::{
        implementation::{ImplementationHead, Index},
        ptc::Context,
    };
    let package = compile(source);
    let context = Context::from_package(&package).unwrap();
    let mut index = Index::default();
    for implementation in package
        .implementations
        .iter()
        .filter(|i| i.interface.is_some())
    {
        index
            .insert(
                ImplementationHead {
                    parameter_types: implementation.parameter_types.clone(),
                    declaration: implementation.declaration,
                    interface: implementation.interface,
                    interface_arguments: implementation.interface_arguments.clone(),
                    subject: implementation.subject.clone(),
                    type_parameters: implementation.type_parameters.clone(),
                    requirements: implementation.requirements.clone(),
                },
                Default::default(),
            )
            .unwrap();
    }
    let world = elaboration::capabilities::World {
        index: &index,
        context: &context,
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: &[],
    };
    let mut items = intrinsic::LangItems::new();
    let protocol = |name| {
        package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration
    };
    items
        .bind_source_traits(
            &package,
            &[
                (intrinsic::LangItem::Copy, protocol("Duplicate")),
                (intrinsic::LangItem::Drop, protocol("Retire")),
            ],
        )
        .unwrap();
    let executable = luna::cleanup::prepare_source(
        &package,
        &world,
        &items,
        iir::InstanceKey {
            declaration: id(&package, "run"),
            arguments: vec![].into(),
        },
        &mut 1_000_000,
        Default::default(),
    )
    .unwrap();
    let mut vm = Interpreter::from_iir(&executable.graph, Budget::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        vm.invoke_instance(&executable.entry, vec![]).unwrap(),
        Value::Bool(true)
    );
    calls(&package, &vm)
}

#[test]
fn synthetic_match_outcomes_destroy_subject_exactly_once() {
    for (value, pattern) in [
        ("Entry {name:tag}", "{name:.ready}"),
        (
            "Choice.item(Entry {name:tag})",
            "Choice.item({name:.ready})",
        ),
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Entry {{name:Symbol}} enum Choice {{empty,item(Entry),}}
            fn entry_clean() {{}}
            impl Retire for Entry {{fn drop(*self) {{entry_clean();}}}}
            fn inspect(tag:Symbol)->bool {{{value} matches {pattern}}}
            fn run()->bool {{inspect(.ready) and not inspect(.other)}}"
        );
        let observed = execute(&source);
        assert_eq!(
            observed
                .iter()
                .filter(|name| name.as_str() == "entry_clean")
                .count(),
            2,
            "{observed:?}"
        );
    }
}

#[test]
fn false_candidate_guards_do_not_destroy_subject_twice() {
    let source="typealias Symbol=.Symbol'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        struct Entry {name:Symbol}
        fn entry_clean() {}
        impl Retire for Entry {fn drop(*self) {entry_clean();}}
        fn inspect(tag:Symbol,enabled:bool)->bool {Entry {name:tag} matches {name:.ready} if enabled}
        fn run()->bool {inspect(.ready,true) and not inspect(.ready,false) and not inspect(.other,true)}";
    let observed = execute(source);
    assert_eq!(
        observed
            .iter()
            .filter(|name| name.as_str() == "entry_clean")
            .count(),
        3,
        "{observed:?}"
    );
}

#[test]
fn implicit_if_is_fallback_retains_cleanup_identity() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        struct Entry {name:Symbol}
        fn entry_clean() {} fn matched() {}
        impl Retire for Entry {fn drop(*self) {entry_clean();}}
        fn inspect(tag:Symbol) {if (Entry {name:tag}) is {name:.ready} do {matched();}}
        fn run()->bool {inspect(.ready);inspect(.other);true}";
    let observed = execute(source);
    assert_eq!(
        observed
            .iter()
            .filter(|name| name.as_str() == "entry_clean")
            .count(),
        2,
        "{observed:?}"
    );
    assert_eq!(
        observed
            .iter()
            .filter(|name| name.as_str() == "matched")
            .count(),
        1,
        "{observed:?}"
    );
}

#[test]
fn spliced_matches_keep_generated_cleanup_sites_hygienic() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        struct Entry {name:Symbol}
        fn entry_clean() {}
        impl Retire for Entry {fn drop(*self) {entry_clean();}}
        comptime fn build()-><input:Symbol|-bool> {'(Entry {name:input} matches {name:.ready})}
        fn run()->bool {
            const code=build();let ready:Symbol=.ready;let other:Symbol=.other;
            ~[input=ready]<code> and not ~[input=other]<code>
        }";
    let observed = execute(source);
    assert_eq!(
        observed
            .iter()
            .filter(|name| name.as_str() == "entry_clean")
            .count(),
        2,
        "{observed:?}"
    );
}

#[test]
fn synthetic_match_cleanup_respects_successful_field_transfers() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Left {} struct Right {} struct Pair {left:Left,right:Right}
        fn left_clean() {} fn right_clean() {}
        impl Retire for Left {fn drop(*self) {left_clean();}}
        impl Retire for Right {fn drop(*self) {right_clean();}}
        fn inspect(flag:bool)->bool {Pair {left:Left {},right:Right {}} matches {left:kept} if flag}
        fn run()->bool {inspect(true) and not inspect(false)}";
    let observed = execute(source);
    let cleanups: Vec<_> = observed
        .iter()
        .filter(|name| name.ends_with("_clean"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        cleanups,
        vec!["left_clean", "right_clean", "right_clean", "left_clean"],
        "{observed:?}"
    );
}

#[test]
fn while_is_cleans_synthetic_failure_and_loop_transfers() {
    for (body, expected) in [
        (
            "active=false;",
            vec!["body_clean", "entry_clean", "entry_clean"],
        ),
        (
            "active=false;continue;",
            vec!["body_clean", "entry_clean", "entry_clean"],
        ),
        ("break;", vec!["body_clean", "entry_clean"]),
        ("return true;", vec!["body_clean", "entry_clean"]),
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Entry {{flag:bool}} struct Body {{}}
            fn entry_clean() {{}} fn body_clean() {{}}
            impl Retire for Entry {{fn drop(*self) {{entry_clean();}}}}
            impl Retire for Body {{fn drop(*self) {{body_clean();}}}}
            fn run()->bool {{var active=true;
                while (Entry {{flag:active}}) is {{flag:true}} do {{let local=Body {{}};{body}}}
                true
            }}"
        );
        let observed = execute(&source);
        let cleanups: Vec<_> = observed
            .iter()
            .filter(|name| name.ends_with("_clean"))
            .map(String::as_str)
            .collect();
        assert_eq!(cleanups, expected, "{body}: {observed:?}");
    }
}

#[test]
fn while_condition_and_named_exits_keep_distinct_cleanup_boundaries() {
    for (condition, body, expected) in [
        (
            "(Entry {flag:active} matches {flag:true})",
            "active=false;continue outer;",
            vec!["entry_clean", "body_clean", "entry_clean"],
        ),
        (
            "(Entry {flag:active} matches {flag:true})",
            "break outer;",
            vec!["entry_clean", "body_clean"],
        ),
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Entry {{flag:bool}} struct Body {{}}
            fn entry_clean() {{}} fn body_clean() {{}}
            impl Retire for Entry {{fn drop(*self) {{entry_clean();}}}}
            impl Retire for Body {{fn drop(*self) {{body_clean();}}}}
            fn run()->bool {{var active=true;
                while:outer {condition} {{let local=Body {{}};{body}}}
                true
            }}"
        );
        let observed = execute(&source);
        let cleanups: Vec<_> = observed
            .iter()
            .filter(|name| name.ends_with("_clean"))
            .map(String::as_str)
            .collect();
        assert_eq!(cleanups, expected, "{body}: {observed:?}");
    }
}

#[test]
fn nested_while_is_named_break_cleans_inner_and_outer_residuals_in_order() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct OuterEntry {flag:bool} struct InnerEntry {flag:bool}
        struct OuterLocal {} struct InnerLocal {}
        fn outer_entry_clean() {} fn inner_entry_clean() {}
        fn outer_local_clean() {} fn inner_local_clean() {}
        impl Retire for OuterEntry {fn drop(*self) {outer_entry_clean();}}
        impl Retire for InnerEntry {fn drop(*self) {inner_entry_clean();}}
        impl Retire for OuterLocal {fn drop(*self) {outer_local_clean();}}
        impl Retire for InnerLocal {fn drop(*self) {inner_local_clean();}}
        fn run()->bool {
            while:outer (OuterEntry {flag:true}) is {flag:true} do {
                let outer_owned=OuterLocal {};
                while (InnerEntry {flag:true}) is {flag:true} do {
                    let inner_owned=InnerLocal {};
                    break outer;
                }
            }
            true
        }";
    let observed = execute(source);
    let cleanups: Vec<_> = observed
        .iter()
        .filter(|name| name.ends_with("_clean"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        cleanups,
        vec![
            "inner_local_clean",
            "inner_entry_clean",
            "outer_local_clean",
            "outer_entry_clean"
        ],
        "{observed:?}"
    );
}

#[test]
fn while_is_transfers_fields_only_for_successful_candidates() {
    for (tail, expected) in [
        (
            "active=false;continue;",
            vec!["left_clean", "right_clean", "right_clean", "left_clean"],
        ),
        ("break;", vec!["left_clean", "right_clean"]),
        ("return true;", vec!["left_clean", "right_clean"]),
    ] {
        let source=format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Left {{}} struct Right {{}} struct Pair {{left:Left,right:Right}}
            fn left_clean() {{}} fn right_clean() {{}}
            impl Retire for Left {{fn drop(*self) {{left_clean();}}}}
            impl Retire for Right {{fn drop(*self) {{right_clean();}}}}
            fn run()->bool {{var active=true;
                while (Pair {{left:Left {{}},right:Right {{}}}}) is {{left:owned}} if active do {{{tail}}}
                true
            }}");
        let observed = execute(&source);
        let cleanups: Vec<_> = observed
            .iter()
            .filter(|name| name.ends_with("_clean"))
            .map(String::as_str)
            .collect();
        assert_eq!(cleanups, expected, "{tail}: {observed:?}");
    }
}

#[test]
fn named_multi_arm_while_preserves_transfers_and_cleanup() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Left {} struct Right {} struct Pair {left:Left,right:Right}
        fn left_clean() {} fn right_clean() {}
        impl Retire for Left {fn drop(*self) {left_clean();}}
        impl Retire for Right {fn drop(*self) {right_clean();}}
        fn run()->bool {var active=true;
            while:scan (Pair {left:Left {},right:Right {}}) is do {
                {left:owned} if active=>{active=false;continue scan;},
                _=>break scan,
            }
            true
        }";
    let observed = execute(source);
    let cleanups: Vec<_> = observed
        .iter()
        .filter(|name| name.ends_with("_clean"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        cleanups,
        vec!["left_clean", "right_clean", "right_clean", "left_clean"],
        "{observed:?}"
    );
}

#[test]
fn multi_arm_while_labels_shadow_and_restore_lexically() {
    let source="trait Duplicate {} trait Retire {fn drop(*self);}
        fn run()->bool {var more=true;var visited=false;
            while:scan more is do {
                true=>{more=false;while:scan true is do {_=>break scan,} visited=true;continue scan;},
                false=>break scan,
            }
            visited
        }";
    execute(source);
    for source in [
        "fn run() {while:scan true is do {_=>break scan,} break scan;}",
        "fn run() {while:scan true is do {_=>{let callback=|| {break scan;};break scan;},}}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn named_multi_arm_while_drives_comptime_code_generation() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        comptime fn build()-><|-bool> {
            var running=true;var code='(false);
            while:scan running is do {
                true=>{code='(true);running=false;continue scan;},
                false=>break scan,
            }
            code
        }
        fn run()->bool {~<build()>}";
    execute(source);
}
