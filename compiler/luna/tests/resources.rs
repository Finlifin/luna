//! Source resource analysis: moves, captures, control flow and call preparation.
#[path = "support/execution.rs"]
mod tuple_support;
use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn memory_summaries_preserve_owned_input_pointer_field_origins() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Link {target:*bool}struct Outer {inner:Link}
        newtype Pointer=*bool;struct Wrapped {target:Pointer}
        struct Generic where T {target:*T}struct Offset {ready:bool,target:*bool}
        fn wrapped(link:Wrapped)->bool {link.target.as(*bool).*}
        fn generic(link:Generic<bool>)->bool {link.target.*}
        fn offset(link:Offset)->bool {link.target.*}
        fn read(link:Link)->bool {link.target.*}
        fn alias(link:Link)->bool {let saved=link;saved.target.*}
        fn nested(link:Outer)->bool {link.inner.target.*}
        fn select(flag:bool,left:Link,right:Link)->bool {
            var selected=left;if flag {selected=right;}selected.target.*
        }
        fn replace(left:Link,right:Link)->bool {
            var selected=left;selected=right;selected.target.*
        }
        fn self_assign(link:Link)->bool {var saved=link;saved=saved;saved.target.*}
        fn project(link:Outer)->bool {let saved=link.inner;saved.target.*}
        fn write(link:Link) {link.target.* = true;}
        fn borrow(link:Link)->*bool {link.target}
        fn forward(link:Link)->bool {read(link)}
        fn forward_nested(link:Outer)->bool {read(link.inner)}
        fn forward_alias(left:Link,right:Link)->bool {let selected=right;forward(selected)}
        fn forward_result(link:Link)->bool {borrow(link).*}
        fn forward_write(link:Link) {write(link);}
        fn reorder(left:Link,right:Link)->bool {select(true,right,left)}
        fn default_pointer(link:Link,.p:*bool=link.target)->bool {p.*}
        fn use_default(link:Link)->bool {default_pointer(link)}",
    );
    for (name, origins) in [
        ("read", vec![(0, vec![0])]),
        ("alias", vec![(0, vec![0])]),
        ("nested", vec![(0, vec![0, 0])]),
        ("wrapped", vec![(0, vec![0])]),
        ("generic", vec![(0, vec![0])]),
        ("offset", vec![(0, vec![1])]),
        ("replace", vec![(1, vec![0])]),
        ("self_assign", vec![(0, vec![0])]),
        ("project", vec![(0, vec![0, 0])]),
        ("select", vec![(1, vec![0]), (2, vec![0])]),
        ("forward", vec![(0, vec![0])]),
        ("forward_nested", vec![(0, vec![0, 0])]),
        ("forward_alias", vec![(1, vec![0])]),
        ("forward_result", vec![(0, vec![0])]),
        ("reorder", vec![(0, vec![0]), (1, vec![0])]),
        ("use_default", vec![(0, vec![0])]),
    ] {
        let id = package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration;
        let summary = memory::infer_in_package(&package, id, &[], &mut 100_000).unwrap();
        let found = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| (a.target.parameter, a.target.input_fields.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(found, origins.into_iter().collect(), "{summary}");
        assert!(
            summary
                .accesses
                .iter()
                .all(|a| a.target.through.is_empty() && a.target.fields.is_empty())
        );
    }
    let write = package
        .functions
        .iter()
        .find(|f| f.name == "forward_write")
        .unwrap();
    let summary = memory::infer_in_package(&package, write.declaration, &[], &mut 100_000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.write && a.target.input_fields == vec![0])
    );
    let id = package
        .functions
        .iter()
        .find(|f| f.name == "borrow")
        .unwrap()
        .declaration;
    let summary = memory::infer_in_package(&package, id, &[], &mut 100_000).unwrap();
    let targets = summary.result_targets.unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets.first().unwrap().input_fields, vec![0]);
    assert!(memory::infer_in_package(&package, id, &[], &mut 1).is_err());
    assert!(memory::infer_in_package(&package, id, &[], &mut 100_000).is_ok());
}

#[test]
fn memory_call_input_paths_are_not_pointee_projections() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Cell {unused:bool,value:bool}
        struct Link {target:*Cell}struct Outer {pad:bool,link:Link}
        fn read(link:Link)->bool {link.target.value.*}
        fn read_pointer(p:*Cell)->bool {p.value.*}
        fn borrow(link:Link)->*Cell {link.target}
        fn forward(outer:Outer)->bool {read(outer.link)}
        fn direct(outer:Outer)->bool {read_pointer(outer.link.target)}
        fn returned(outer:Outer)->bool {borrow(outer.link).value.*}",
    );
    for name in ["forward", "direct", "returned"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100_000).unwrap();
        let accesses = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .collect::<Vec<_>>();
        assert_eq!(accesses.len(), 1, "{summary}");
        let target = &accesses[0].target;
        assert_eq!(target.parameter, 0);
        assert_eq!(target.input_fields, vec![1, 0], "{summary}");
        assert!(target.through.is_empty(), "{summary}");
        assert_eq!(
            target
                .fields
                .iter()
                .map(|(_, index)| *index)
                .collect::<Vec<_>>(),
            vec![1],
            "{summary}"
        );
    }
}

#[test]
fn aggregate_pointer_calls_check_actual_storage_lifetimes() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, accepted) in [
        (
            "let value=Cell {value:false};let seen=read(Link {target:value.ref});",
            true,
        ),
        (
            "let value=Cell {value:false};write(do {Link {target:value.ref}});",
            true,
        ),
        (
            "let value=Cell {value:false};let seen=late_read(Link {target:value.ref},do {let moved=value;false});",
            false,
        ),
        (
            "let value=Cell {value:false};let link=Link {target:value.ref};let seen=read(link);",
            true,
        ),
        (
            "let value=Cell {value:false};let link=Link {target:value.ref};write(link);",
            true,
        ),
        (
            "let value=Cell {value:false};let link=Link {target:value.ref};let seen=borrow(link).value.*;",
            true,
        ),
        (
            "let value=Cell {value:false};let link=Link {target:value.ref};let seen=default_read(link);",
            true,
        ),
        (
            "let value=Cell {value:false};let link=Link {target:value.ref};let moved=value;let seen=read(link);",
            false,
        ),
        (
            "let value=Cell {value:false};let link=Link {target:value.ref};let seen=late_read(link,do {let moved=value;false});",
            false,
        ),
        (
            "let link=do {let value=Cell {value:false};Link {target:value.ref}};let seen=read(link);",
            false,
        ),
    ] {
        let source = format!(
            "struct Cell {{value:bool}}struct Link {{target:*Cell}}
            fn read(link:Link)->bool {{link.target.value.*}}
            fn write(link:Link) {{link.target.value.* = true;}}
            fn borrow(link:Link)->*Cell {{link.target}}
            fn default_read(link:Link,.p:*Cell=link.target)->bool {{p.value.*}}
            fn late_read(link:Link,flag:bool)->bool {{link.target.value.*}}
            fn run() {{{body}}}"
        );
        let package = compile(&source);
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
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
        );
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
    }
}

#[test]
fn owned_pointer_field_helpers_execute_with_checked_cleanup_premises() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Link {target:*bool}
        fn write(link:Link) {link.target.* = true;}
        fn read(link:Link)->bool {link.target.*}
        fn borrow(link:Link)->*bool {link.target}
        fn default_read(link:Link,.p:*bool=link.target)->bool {p.*}
        fn via(link:Link,f:fn(Link)->bool)->bool {f(link)}
        fn run()->bool {let value=false;write(Link {target:value.ref});
            read(Link {target:value.ref}) and default_read(Link {target:value.ref})
            and borrow(Link {target:value.ref}).* and via(Link {target:value.ref},read)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn owned_pointer_field_premises_are_explicit_and_layout_checked() {
    use elaboration::verification::moves::{self, BorrowedField, Delivery};
    let package = compile("struct Link {target:*bool}fn read(link:Link)->bool {link.target.*}");
    let function = package.functions.iter().find(|f| f.name == "read").unwrap();
    let delivery = |ty: &nir::Term| {
        Ok(
            if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            },
        )
    };
    assert!(moves::analyze_in_package(&package, function, delivery, 100_000).is_err());
    let forwarded = compile(
        "struct Link {target:*bool}fn read(link:Link)->bool {link.target.*}fn forward(link:Link)->bool {read(link)}",
    );
    let forward = forwarded
        .functions
        .iter()
        .find(|f| f.name == "forward")
        .unwrap();
    assert!(moves::analyze_in_package(&forwarded, forward, delivery, 100_000).is_err());
    let field = BorrowedField {
        parameter: 0,
        fields: vec![0],
        pointee: nir::Term::Primitive(nir::Primitive::Bool),
    };
    let analyze = |fields: &[BorrowedField]| {
        moves::analyze_in_package_with_field_context(
            &package,
            function,
            delivery,
            100_000,
            None,
            &[],
            &[],
            &[],
            vec![],
            fields,
        )
    };
    let report = analyze(std::slice::from_ref(&field)).unwrap();
    assert_eq!(report.borrowed_fields, vec![field.clone()]);
    assert!(
        report
            .to_string()
            .contains("(borrowed-field (parameter 0) (fields 0)")
    );
    assert!(analyze(&[field.clone(), field.clone()]).is_err());
    for bad in [
        BorrowedField {
            fields: vec![],
            ..field.clone()
        },
        BorrowedField {
            fields: vec![1],
            ..field.clone()
        },
        BorrowedField {
            parameter: 1,
            ..field.clone()
        },
        BorrowedField {
            pointee: nir::Term::Primitive(nir::Primitive::Void),
            ..field
        },
    ] {
        assert!(analyze(&[bad]).is_err());
    }
}

#[test]
fn pointer_field_callback_factories_keep_checked_input_origins() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Link {target:*bool}
        fn yes()->bool {true}fn no()->bool {false}
        fn choose(link:Link)->fn()->bool {if link.target.* {yes}else {no}}
        fn relay(link:Link)->fn()->bool {choose(link)}
        fn typed(comptime T:.Type'builtin,link:Link)->fn()->bool {choose(link)}
        fn staged(comptime pick:bool,link:Link)->fn()->bool {if pick {choose(link)}else {no}}
        fn apply(link:Link,f:fn(Link)->fn()->bool)->bool {let callback=f(link);callback()}";
    for (body, expected) in [
        (
            "let flag=true;let a=staged(true,Link {target:flag.ref});let b=staged(false,Link {target:flag.ref});a() and not b()",
            true,
        ),
        (
            "let flag=true;let callback=typed(bool,Link {target:flag.ref});callback()",
            true,
        ),
        (
            "let flag=true;let callback=choose(Link {target:flag.ref});callback()",
            true,
        ),
        (
            "let flag=false;let callback=relay(Link {target:flag.ref});callback()",
            false,
        ),
        ("let flag=true;apply(Link {target:flag.ref},relay)", true),
    ] {
        let package = compile(&format!("{prefix}fn run()->bool {{{body}}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(expected)
        );
    }
    let package = compile(&format!(
        "{prefix}fn run()->bool {{let link=do {{let flag=true;Link {{target:flag.ref}}}};let callback=relay(link);callback()}}"
    ));
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .is_err()
    );
}

#[test]
fn constructed_pointer_fields_snapshot_initializers_and_project_through_wrappers() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Link {target:*bool}struct Outer {inner:Link}
        struct Tagged {target:*bool,marker:bool}newtype Wrapped=Link;
        fn read(link:Link)->bool {link.target.*}
        fn built(p:*bool)->bool {read(Link {target:p})}
        fn nested(p:*bool)->bool {let outer=Outer {inner:Link {target:p}};outer.inner.target.*}
        fn projected(p:*bool)->bool {(Link {target:p}).target.*}
        fn wrapped(p:*bool)->bool {let value=(Link {target:p}).as(Wrapped);value.as(Link).target.*}
        fn snapshot(p:*bool,q:*bool)->bool {var selected=p;
            let tagged=Tagged {target:selected,marker:do {selected=q;false}};tagged.target.*}
        fn run()->bool {let first=true;let second=false;built(first.ref) and nested(first.ref)
            and projected(first.ref) and wrapped(first.ref) and snapshot(first.ref,second.ref)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn constructed_pointer_cleanup_cannot_hide_a_dead_call_target() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Holder {target:*Cell}impl Retire for Holder {fn drop(*self) {let target=self.target.*;target.flag.* = true;}}
        fn retire(p:*Cell) {let holder=Holder {target:p};}
        fn make_holder(p:*Cell)->Holder {Holder {target:p}}
        fn relay_holder(p:*Cell)->Holder {make_holder(p)}
        fn returned(p:*Cell) {let holder=relay_holder(p);}
        fn discarded_return(p:*Cell) {relay_holder(p);}
        fn consume_holder(holder:Holder) {}
        fn forwarded_return(p:*Cell) {consume_holder(relay_holder(p));}
        fn captured_return(p:*Cell) {let make=|| Holder {target:p};let holder=make();}
        fn argument_return(p:*Cell) {let make=|q:*Cell| Holder {target:q};let holder=make(p);}
        fn immediate_return(p:*Cell) {let holder=(|| Holder {target:p})();}
        fn discarded_closure_return(p:*Cell) {let make=|| Holder {target:p};make();}
        fn relay(p:*Cell) {retire(p);}
        fn noop() {}
        fn apply(p:*Cell,done:fn()) {let holder=Holder {target:p};done();}
        fn higher(p:*Cell) {apply(p,noop);}
        fn immediate(p:*Cell) {(|| {let holder=Holder {target:p};})();}
        fn closed(p:*Cell) {let callback=|| {let holder=Holder {target:p};};callback();}
        fn closure_argument(p:*Cell) {let callback=|q:*Cell| {let holder=Holder {target:q};};callback(p);}";
    for helper in [
        "retire",
        "returned",
        "discarded_return",
        "forwarded_return",
        "captured_return",
        "argument_return",
        "immediate_return",
        "discarded_closure_return",
        "relay",
        "higher",
        "closure_argument",
        "immediate",
        "closed",
    ] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let cell=Cell {{flag:false}};{helper}(cell.ref);cell.flag}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{helper}: {e}")),
            comptime_interpretor::Value::Bool(true),
            "{helper}"
        );
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let cell=Cell {{flag:false}};let pointer=cell.ref;let moved=cell;{helper}(pointer);true}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{helper}: {error}"
        );
    }
}

#[test]
fn acquired_cleanup_footprints_are_keyed_validated_and_displayed() {
    use elaboration::verification::memory;
    let mut package = compile("fn helper(p:*bool) {}fn relay(p:*bool) {helper(p);}");
    let helper = package
        .functions
        .iter()
        .find(|f| f.name == "helper")
        .unwrap()
        .clone();
    let relay = package
        .functions
        .iter()
        .find(|f| f.name == "relay")
        .unwrap()
        .declaration;
    package
        .runtime_instances
        .push(nir::program::RuntimeInstance {
            declaration: helper.declaration,
            arguments: vec![],
            function: helper,
            cleanup_accesses: vec![nir::program::RuntimeCleanupAccess {
                parameter: 0,
                input_fields: vec![],
                through: vec![],
                conditions: Default::default(),
                fields: vec![],
                write: true,
                read_value: true,
                ty: nir::Term::Primitive(nir::Primitive::Bool),
            }],
        });
    let summary = memory::infer_in_package(&package, relay, &[], &mut 10000).unwrap();
    assert_eq!(summary.accesses.len(), 1);
    assert!(summary.accesses[0].write);
    assert!(
        package
            .to_string()
            .contains("(cleanup-access (parameter 0)")
    );
    assert!(memory::infer_in_package(&package, relay, &[], &mut 0).is_err());
    assert_eq!(
        memory::infer_in_package(&package, relay, &[], &mut 10000).unwrap(),
        summary
    );
    package.runtime_instances[0].cleanup_accesses[0].parameter = 1;
    assert!(
        memory::infer_in_package(&package, relay, &[], &mut 10000)
            .unwrap_err()
            .contains("parameter is missing")
    );
    package.runtime_instances[0].cleanup_accesses[0].parameter = 0;
    package.runtime_instances[0].cleanup_accesses[0].ty = nir::Term::Primitive(nir::Primitive::U8);
    assert!(
        memory::infer_in_package(&package, relay, &[], &mut 10000)
            .unwrap_err()
            .contains("target type mismatch")
    );
}

#[test]
fn stored_pointer_closures_snapshot_and_separate_candidate_origins() {
    use elaboration::verification::memory;
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        struct Pair {left:bool,right:bool}
        fn alias(p:*bool)->bool {let callback=||p.*;let saved=callback;saved()}
        fn snapshot(p:*bool,q:*bool)->bool {var selected=p;let callback=||selected.*;selected=q;callback()}
        fn choose(flag:bool,p:*bool,q:*bool)->*bool {let callback=if flag {||p} else {||q};callback()}
        fn branch(flag:bool,a:*Pair,b:*Pair)->bool {let callback=if flag {||a.left.*} else {||b.right.*};callback()}
        fn peek(p:*bool)->bool {p.*}
        fn clear(p:*bool)->bool {p.* = false;false}
        fn evolving(p:*bool)->bool {var action=peek;let callback=|| {let result=action(p);action=clear;result};let first=callback();let second=callback();first and not second}
        fn run()->bool {let first=true;let second=false;let a=Pair {left:true,right:false};let b=Pair {left:false,right:true};
            alias(first.ref) and snapshot(first.ref,second.ref) and choose(true,first.ref,second.ref).*
            and not choose(false,first.ref,second.ref).* and branch(true,a.ref,b.ref) and branch(false,a.ref,b.ref) and evolving(first.ref)}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for name in ["alias", "snapshot", "choose", "branch", "evolving"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err());
        assert_eq!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap(),
            summary
        );
        if name == "evolving" {
            assert!(summary.accesses.iter().any(|a| a.write), "{summary}");
        }
        if name == "snapshot" {
            assert!(
                summary.accesses.iter().all(|a| a.target.parameter == 0),
                "{summary}"
            );
        }
        if name == "branch" {
            let reads = summary
                .accesses
                .iter()
                .filter(|a| a.read_value)
                .map(|a| {
                    (
                        a.target.parameter,
                        a.target.fields.iter().map(|(_, i)| *i).collect::<Vec<_>>(),
                    )
                })
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                reads,
                std::collections::BTreeSet::from([(1, vec![0]), (2, vec![1])]),
                "{summary}"
            );
        }
    }
    let rebound = compile(
        "fn inspect(p:*bool,q:*bool)->bool {var current=p;let callback=|| {current=q;current.*};callback()}",
    );
    let function = rebound
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&rebound, function.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.read_value && access.target.parameter == 1),
        "{summary}"
    );
    assert!(
        summary
            .accesses
            .iter()
            .all(|access| access.target.parameter == 1),
        "{summary}"
    );
}

#[test]
fn conditional_aggregate_pointer_results_keep_branch_origins() {
    use elaboration::verification::memory;
    let prefix="trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}struct Outer {inner:Link}newtype Wrapped=Link;
        fn read(link:Link)->bool {link.target.*}
        fn pick(flag:bool,p:*bool,q:*bool)->bool {let link=if flag {Link {target:p}} else {Link {target:q}};read(link)}
        fn matched(flag:bool,p:*bool,q:*bool)->bool {let link=flag match {true=>Link {target:p},false=>Link {target:q}};link.target.*}
        fn nested(flag:bool,p:*bool,q:*bool)->bool {let outer=if flag {Outer {inner:Link {target:p}}} else {Outer {inner:Link {target:q}}};outer.inner.target.*}
        fn wrapped(flag:bool,p:*bool,q:*bool)->bool {let value=if flag {(Link {target:p}).as(Wrapped)} else {(Link {target:q}).as(Wrapped)};value.as(Link).target.*}
        fn saved(flag:bool,p:*bool,q:*bool)->bool {var selected=p;let link=if flag {let saved=Link {target:selected};selected=q;saved} else {Link {target:p}};selected=q;link.target.*}
        fn guarded(flag:bool,p:*bool,q:*bool)->bool {let link=flag match {_ if false=>Link {target:q},_=>Link {target:p}};link.target.*}
        fn early(flag:bool,p:*bool,q:*bool)->bool {let link=if flag {return false;} else {Link {target:p}};link.target.*}
        fn early_match(flag:bool,p:*bool,q:*bool)->bool {let link=flag match {true=>return false,false=>Link {target:p}};link.target.*}";
    let package=compile(&format!("{prefix} fn run()->bool {{let first=true;let second=false;
        pick(true,first.ref,second.ref) and not pick(false,first.ref,second.ref)
        and matched(true,first.ref,second.ref) and not matched(false,first.ref,second.ref)
        and nested(true,first.ref,second.ref) and not nested(false,first.ref,second.ref)
        and wrapped(true,first.ref,second.ref) and not wrapped(false,first.ref,second.ref)
        and saved(true,first.ref,second.ref) and saved(false,first.ref,second.ref) and guarded(false,first.ref,second.ref) and early(false,first.ref,second.ref) and early_match(false,first.ref,second.ref)}}"));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for name in [
        "pick",
        "matched",
        "nested",
        "wrapped",
        "guarded",
        "saved",
        "early",
        "early_match",
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let roots = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        let expected = if ["guarded", "saved", "early", "early_match"].contains(&name) {
            std::collections::BTreeSet::from([1])
        } else {
            std::collections::BTreeSet::from([1, 2])
        };
        assert_eq!(roots, expected, "{name}: {summary}");
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err());
        assert_eq!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap(),
            summary
        );
    }
    for name in ["pick", "matched", "nested", "wrapped"] {
        let invalid = compile(&format!(
            "{prefix} fn run()->bool {{let live=true;let expired=do {{let local=false;local.ref}};{name}(false,live.ref,expired)}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{name}: {error}"
        );
    }
}

#[test]
fn conditional_owned_inputs_preserve_pointer_field_paths() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Input {left:*bool,right:*bool}struct Link {target:*bool}
        fn choose(flag:bool,a:Input,b:Input)->bool {
            let link=if flag {Link {target:a.left}} else {Link {target:b.right}};
            link.target.*
        }
        fn matched(flag:bool,a:Input,b:Input)->bool {
            let link=flag match {true=>Link {target:a.left},false=>Link {target:b.right}};
            link.target.*
        }";
    for name in ["choose", "matched"] {
        for flag in ["true", "false"] {
            let package = compile(&format!(
                "{prefix} fn run()->bool {{let yes=true;let no=false;
                {name}({flag},Input {{left:yes.ref,right:no.ref}},Input {{left:no.ref,right:yes.ref}})}}"
            ));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    200_000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(true),
                "{name}({flag})"
            );
            let function = package.functions.iter().find(|f| f.name == name).unwrap();
            let summary =
                memory::infer_in_package(&package, function.declaration, &[], &mut 100_000)
                    .unwrap();
            let reads = summary
                .accesses
                .iter()
                .filter(|access| access.read_value)
                .map(|access| (access.target.parameter, access.target.input_fields.clone()))
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                reads,
                std::collections::BTreeSet::from([(1, vec![0]), (2, vec![1])]),
                "{summary}"
            );

            let (left, right) = if flag == "true" {
                ("expired", "live.ref")
            } else {
                ("live.ref", "expired")
            };
            let invalid = compile(&format!(
                "{prefix} fn run()->bool {{let live=true;let expired=do {{let local=false;local.ref}};
                {name}({flag},Input {{left:{left},right:live.ref}},Input {{left:live.ref,right:{right}}})}}"
            ));
            let error = luna::execution::interpret_with_protocols(
                &invalid,
                "run",
                "Duplicate",
                "Retire",
                200_000,
            )
            .unwrap_err();
            assert!(
                error.contains("cannot prepare resource-checked execution"),
                "{name}({flag}): {error}"
            );
        }
    }
}

#[test]
fn owned_pointer_patterns_keep_subject_origins() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}struct Outer {inner:Link}
        fn bound(link:Link)->bool {let {target}=link;target.*}
        fn matched(link:Link)->bool {link match {{target}=>target.*}}
        fn whole(link:Link)->bool {link match {saved=>saved.target.*}}
        fn nested(outer:Outer)->bool {let {inner:{target}}=outer;target.*}
        fn nested_match(outer:Outer)->bool {outer match {{inner:{target}}=>target.*}}";
    for (name, nested) in [
        ("bound", false),
        ("matched", false),
        ("whole", false),
        ("nested", true),
        ("nested_match", true),
    ] {
        let argument = |target: &str| {
            let link = format!("Link {{target:{target}}}");
            if nested {
                format!("Outer {{inner:{link}}}")
            } else {
                link
            }
        };
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let yes=true;{name}({})}}",
            argument("yes.ref")
        ));
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100_000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| (a.target.parameter, a.target.input_fields.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            reads,
            std::collections::BTreeSet::from([(0, if nested { vec![0, 0] } else { vec![0] })]),
            "{name}: {summary}"
        );
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err());
        assert_eq!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 100_000).unwrap(),
            summary
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{name}"
        );
        let invalid = compile(&format!(
            "{prefix} fn run()->bool {{let expired=do {{let local=false;local.ref}};{name}({})}}",
            argument("expired")
        ));
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{name}: {error}"
        );
    }
}

#[test]
fn owned_pointer_or_patterns_merge_remapped_field_origins() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Choice {flag:bool,left:*bool,right:*bool}
        fn choose(value:Choice)->bool {
            value match {({flag:true,left:p} or {flag:false,right:p})=>p.*}
        }
        fn bound(value:Choice)->bool {
            let ({flag:true,left:p} or {flag:false,right:p})=value else {return false;};p.*
        }
        fn whole(value:Choice)->bool {
            value match {({flag:true} or {flag:false}) as saved=>saved.left.*}
        }";
    for name in ["choose", "bound", "whole"] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let yes=true;
            {name}(Choice {{flag:true,left:yes.ref,right:yes.ref}})
            and {name}(Choice {{flag:false,left:yes.ref,right:yes.ref}})}}"
        ));
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100_000).unwrap();
        let fields = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.input_fields.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let expected = if name == "whole" {
            vec![vec![1]]
        } else {
            vec![vec![1], vec![2]]
        };
        assert_eq!(fields, expected.into_iter().collect(), "{name}: {summary}");
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{name}"
        );
    }
}

#[test]
fn named_aggregate_returns_preserve_pointer_field_origins() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}struct Outer {inner:Link}newtype Wrapped=Link;
        fn make(p:*bool)->Link {Link {target:p}}
        fn relay(p:*bool)->Link {make(p)}
        fn forward(link:Link)->Link {link}
        fn recursive(flag:bool,p:*bool)->Link {if flag {make(p)} else {recursive(true,p)}}
        fn explicit(p:*bool)->Link {return Link {target:p};}
        fn select(flag:bool,p:*bool,q:*bool)->Link {
            if flag {return make(p);} Link {target:q}
        }
        fn nested(p:*bool)->Outer {Outer {inner:relay(p)}}
        fn wrapped(p:*bool)->Wrapped {(make(p)).as(Wrapped)}";
    for (name, path, call, access) in [
        ("make", vec![0], "make(yes.ref)", "target.*"),
        ("relay", vec![0], "relay(yes.ref)", "target.*"),
        ("forward", vec![0], "forward(make(yes.ref))", "target.*"),
        ("recursive", vec![0], "recursive(false,yes.ref)", "target.*"),
        ("explicit", vec![0], "explicit(yes.ref)", "target.*"),
        (
            "select",
            vec![0],
            "select(false,no.ref,yes.ref)",
            "target.*",
        ),
        ("nested", vec![0, 0], "nested(yes.ref)", "inner.target.*"),
        (
            "wrapped",
            vec![0, 0],
            "wrapped(yes.ref)",
            "as(Link).target.*",
        ),
    ] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let yes=true;let no=false;let result={call};result.{access}}}"
        ));
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100_000).unwrap();
        let origins = summary.pointer_fields.get(&path).unwrap().as_ref().unwrap();
        let parameters = origins
            .iter()
            .map(|p| p.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            parameters,
            if name == "select" {
                [1, 2].into_iter().collect()
            } else if name == "recursive" {
                [1].into_iter().collect()
            } else {
                [0].into_iter().collect()
            },
            "{name}: {summary}"
        );
        if name == "forward" {
            assert!(origins.iter().all(|origin| origin.input_fields == vec![0]));
        }
        assert!(summary.to_string().contains("(pointer-fields (leaf (path"));
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err());
        assert_eq!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 100_000).unwrap(),
            summary
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{name}"
        );
        let expired_call = call.replace("yes.ref", "expired");
        let invalid = compile(&format!(
            "{prefix} fn run()->bool {{let no=false;let expired=do {{let local=true;local.ref}};let result={expired_call};result.{access}}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{name}: {error}"
        );
    }
    let invalid = compile(
        "struct Link {target:*bool} fn escape()->Link {let local=true;Link {target:local.ref}}",
    );
    let function = invalid
        .functions
        .iter()
        .find(|f| f.name == "escape")
        .unwrap();
    assert!(memory::infer_in_package(&invalid, function.declaration, &[], &mut 100_000).is_err());
}

#[test]
fn local_named_aggregate_returns_merge_candidate_origins() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}
        fn left(p:*bool,q:*bool)->Link {Link {target:p}}
        fn right(p:*bool,q:*bool)->Link {Link {target:q}}
        fn single(p:*bool,q:*bool)->bool {let action=left;let result=action(p,q);result.target.*}
        fn choose(flag:bool,p:*bool,q:*bool)->bool {
            let action=if flag {left} else {right};let result=action(p,q);result.target.*
        }";
    let package=compile(&format!("{prefix} fn run()->bool {{let yes=true;let no=false;
        single(yes.ref,no.ref) and choose(true,yes.ref,no.ref) and not choose(false,yes.ref,no.ref)}}"));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "choose")
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
        .filter(|a| a.read_value)
        .map(|a| a.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, [1, 2].into_iter().collect());
    for call in [
        "single(expired,live.ref)",
        "choose(true,expired,live.ref)",
        "choose(false,live.ref,expired)",
    ] {
        let invalid = compile(&format!(
            "{prefix} fn run()->bool {{let live=true;let expired=do {{let local=false;local.ref}};{call}}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{call}: {error}"
        );
    }
}

#[test]
fn temporary_named_aggregate_calls_keep_origins_and_evaluation_order() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}struct Factory {make:fn(*bool,*bool)->Link}
        fn left(p:*bool,q:*bool)->Link {Link {target:p}}
        fn right(p:*bool,q:*bool)->Link {Link {target:q}}
        fn selected(flag:bool,p:*bool,q:*bool)->bool {
            let result=(if flag {left} else {right})(p,q);result.target.*
        }
        fn projected(p:*bool,q:*bool)->bool {
            let result=(Factory {make:left}).make(p,q);result.target.*
        }
        fn ordered(p:*bool,q:*bool)->bool {
            var before=false;
            let result=(do {before=not before;left})(do {if not before {return false;}p},q);
            result.target.*
        }";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let yes=true;let no=false;
        selected(true,yes.ref,no.ref) and not selected(false,yes.ref,no.ref)
        and projected(yes.ref,no.ref) and ordered(yes.ref,no.ref)}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for call in [
        "selected(true,expired,live.ref)",
        "selected(false,live.ref,expired)",
        "projected(expired,live.ref)",
        "ordered(expired,live.ref)",
    ] {
        let invalid = compile(&format!(
            "{prefix} fn run()->bool {{let live=true;let expired=do {{let local=false;local.ref}};{call}}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{call}: {error}"
        );
    }
}

#[test]
fn aggregate_result_origins_survive_until_read_and_cleanup() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Link {target:*Cell}
        fn make(p:*Cell)->Link {Link {target:p}}
        fn relay(p:*Cell)->Link {make(p)}
        fn local(p:*Cell)->Link {let action=make;action(p)}
        fn selected(p:*Cell)->Link {let action=if true {make} else {relay};action(p)}
        fn temporary(p:*Cell)->Link {(do {make})(p)}
        fn captured(p:*Cell)->Link {let build=|| Link {target:p};build()}
        fn argument(p:*Cell)->Link {let build=|q:*Cell| Link {target:q};build(p)}
        fn immediate(p:*Cell)->Link {(|| Link {target:p})()}
        fn early(p:*Cell)->Link {let build=|| {return Link {target:p};};build()}";
    for name in [
        "make",
        "relay",
        "local",
        "selected",
        "temporary",
        "captured",
        "argument",
        "immediate",
        "early",
    ] {
        for moved in [false, true] {
            let invalidate = if moved { "let moved=cell;" } else { "" };
            let package = compile(&format!(
                "{prefix} fn run()->bool {{let cell=Cell {{flag:true}};
                let result={name}(cell.ref);{invalidate}result.target.flag.*}}"
            ));
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000,
            );
            if moved {
                let error = result.unwrap_err();
                assert!(
                    error.contains("cannot prepare resource-checked execution"),
                    "read {name}: {error}"
                );
            } else {
                assert_eq!(
                    result.unwrap(),
                    comptime_interpretor::Value::Bool(true),
                    "read {name}"
                );
            }

            // The negative case performs no explicit read after the move: only
            // the returned owner's destructor can expose the stale dependency.
            let result_expression = if moved { "true" } else { "cell.flag" };
            let package=compile(&format!("{prefix}
                impl Retire for Link {{fn drop(*self) {{let target=self.target.*;target.flag.* = true;}}}}
                fn run()->bool {{let cell=Cell {{flag:false}};
                    do {{let result={name}(cell.ref);{invalidate}}};{result_expression}}}"));
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000,
            );
            if moved {
                let error = result.unwrap_err();
                assert!(
                    error.contains("cannot prepare resource-checked execution"),
                    "cleanup {name}: {error}"
                );
            } else {
                assert_eq!(
                    result.unwrap(),
                    comptime_interpretor::Value::Bool(true),
                    "cleanup {name}"
                );
            }
        }
    }
}

#[test]
fn closure_aggregate_returns_preserve_arguments_captures_and_branches() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Link {target:*bool}
        struct Outer {link:Link}
        struct Mixed {target:*bool,action:fn(*bool)->*bool}
        fn relay_pointer(p:*bool)->*bool {p}
        fn argument(flag:bool,p:*bool,q:*bool)->Link {let make=|target:*bool| Link {target:target};make(p)}
        fn captured(flag:bool,p:*bool,q:*bool)->Link {let make=|| Link {target:p};make()}
        fn owned_capture(flag:bool,p:*bool,q:*bool)->Link {let link=Link {target:p};let make=|| Link {target:link.target};make()}
        fn owned_immediate(flag:bool,p:*bool,q:*bool)->Link {let link=Link {target:p};(|| Link {target:link.target})()}
        fn owned_nested(flag:bool,p:*bool,q:*bool)->Link {let outer=Outer {link:Link {target:p}};let make=|| Link {target:outer.link.target};make()}
        fn owned_mixed(flag:bool,p:*bool,q:*bool)->Link {let mixed=Mixed {target:p,action:relay_pointer};let make=|| Link {target:mixed.action(mixed.target)};make()}
        fn owned_snapshot(flag:bool,p:*bool,q:*bool)->Link {var current=p;let link=Link {target:current};let make=|| Link {target:link.target};current=q;make()}
        fn owned_selected(flag:bool,p:*bool,q:*bool)->Link {let left=Link {target:p};let right=Link {target:q};(if flag {|| Link {target:left.target}} else {|| Link {target:right.target}})()}
        fn immediate(flag:bool,p:*bool,q:*bool)->Link {(|| Link {target:p})()}
        fn early(flag:bool,p:*bool,q:*bool)->Link {let make=|| {if flag {return Link {target:p};}Link {target:q}};make()}
        fn selected(flag:bool,p:*bool,q:*bool)->Link {(if flag {|| Link {target:p}} else {|| Link {target:q}})()}";
    for (name, flag, first) in [
        ("argument", true, true),
        ("captured", true, true),
        ("owned_capture", true, true),
        ("owned_immediate", true, true),
        ("owned_nested", true, true),
        ("owned_mixed", true, true),
        ("owned_snapshot", true, true),
        ("owned_selected", true, true),
        ("owned_selected", false, false),
        ("immediate", true, true),
        ("early", true, true),
        ("early", false, false),
        ("selected", true, true),
        ("selected", false, false),
    ] {
        for moved in [false, true] {
            let invalidate = if !moved {
                ""
            } else if first {
                "let moved=first;"
            } else {
                "let moved=second;"
            };
            let package=compile(&format!("{prefix} fn run()->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                let result={name}({flag},first.flag.ref,second.flag.ref);{invalidate}result.target.*}}"));
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000,
            );
            if moved {
                let error = result.unwrap_err();
                assert!(
                    error.contains("cannot prepare resource-checked execution"),
                    "{name}({flag}): {error}"
                );
            } else {
                use elaboration::verification::memory;
                let function = package.functions.iter().find(|f| f.name == name).unwrap();
                let summary =
                    memory::infer_in_package(&package, function.declaration, &[], &mut 100_000)
                        .unwrap();
                let fields = summary
                    .pointer_fields
                    .get(&vec![0])
                    .unwrap()
                    .as_ref()
                    .unwrap();
                let roots = fields
                    .iter()
                    .map(|target| target.parameter)
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(
                    roots,
                    if ["early", "selected", "owned_selected"].contains(&name) {
                        [1, 2].into_iter().collect()
                    } else {
                        [1].into_iter().collect()
                    },
                    "{name}: {summary}"
                );
                assert!(
                    memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err()
                );
                assert_eq!(
                    memory::infer_in_package(&package, function.declaration, &[], &mut 100_000)
                        .unwrap(),
                    summary
                );
                assert_eq!(
                    result.unwrap_or_else(|error| panic!("{name}({flag}): {error}")),
                    comptime_interpretor::Value::Bool(first)
                );
            }
        }
    }
    let invalid = compile(&format!(
        "{prefix} fn escape()->Link {{(|| {{let local=true;Link {{target:local.ref}}}})()}}"
    ));
    let function = invalid
        .functions
        .iter()
        .find(|f| f.name == "escape")
        .unwrap();
    let error = elaboration::verification::memory::infer_in_package(
        &invalid,
        function.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap_err();
    assert!(
        error.contains("local memory target cannot escape"),
        "{error}"
    );
}

#[test]
fn aggregate_pointer_captures_check_accesses_and_reject_storage_addresses() {
    use elaboration::verification::memory;
    for immediate in [false, true] {
        let invoke = if immediate {
            "(|| {link.target.* = true;link.target.*})()"
        } else {
            "let action=|| {link.target.* = true;link.target.*};action()"
        };
        let prefix = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            struct Link {{target:*bool}}
            fn write(p:*bool)->bool {{let link=Link {{target:p}};{invoke}}}"
        );
        for moved in [false, true] {
            let invalidation = if moved { "let moved=cell;" } else { "" };
            let package = compile(&format!(
                "{prefix} fn run()->bool {{let cell=Cell {{flag:false}};let pointer=cell.flag.ref;{invalidation}write(pointer)}}"
            ));
            let function = package
                .functions
                .iter()
                .find(|f| f.name == "write")
                .unwrap();
            let summary =
                memory::infer_in_package(&package, function.declaration, &[], &mut 100_000)
                    .unwrap();
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|access| access.target.parameter == 0 && access.write),
                "{summary}"
            );
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|access| access.target.parameter == 0 && access.read_value),
                "{summary}"
            );
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000,
            );
            if moved {
                assert!(
                    result
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
            }
        }
    }
    for (body, expected) in [(
        "let slot=link.target.ref;link.target.*",
        "addressing captured pointer storage",
    )] {
        let package = compile(&format!(
            "struct Link {{target:*bool}}fn inspect(p:*bool,q:*bool)->bool {{var link=Link {{target:p}};let action=|| {{{body}}};action()}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let error = memory::infer_in_package(&package, function.declaration, &[], &mut 100_000)
            .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn aggregate_defaults_keep_prepared_parameter_pointer_origins() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Link {target:*bool}struct Outer {inner:Link}
        pure fn make(p:*bool)->Link {Link {target:p}}
        fn direct(p:*bool,.link:Link=do {Link {target:p}})->bool {link.target.*}
        fn factory(p:*bool,.link:Link=make(p))->bool {link.target.*}
        fn nested(p:*bool,.outer:Outer=do {Outer {inner:Link {target:p}}})->bool {outer.inner.target.*}
        fn chained(p:*bool,.link:Link=do {Link {target:p}},.q:*bool=link.target)->bool {q.*}
        fn later(p:*bool,ignored:bool,.link:Link=do {Link {target:p}})->bool {link.target.*}";
    for name in ["direct", "factory", "nested", "chained"] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let cell=Cell {{flag:true}};{name}(cell.flag.ref)}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap_or_else(|e| panic!("{name}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = compile(&format!(
            "{prefix} fn run()->bool {{let cell=Cell {{flag:true}};let pointer=cell.flag.ref;let moved=cell;{name}(pointer)}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{name}: {error}"
        );
    }
    let snapshot = compile(&format!(
        "{prefix} fn run()->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
        var selected=first.flag.ref;later(selected,do {{selected=second.flag.ref;true}})}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&snapshot, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = compile(&format!(
        "{prefix} fn run()->bool {{let first=Cell {{flag:true}};
        later(first.flag.ref,do {{let moved=first;true}})}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&invalid, "run", "Duplicate", "Retire", 200_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

fn compile(source: &str) -> nir::program::Package {
    compile_in(source, 0)
}
fn compile_in(source: &str, identity: u64) -> nir::program::Package {
    compile_in_with_resources(source, identity, None, None)
}
fn compile_in_with_resources(
    source: &str,
    identity: u64,
    copy: Option<&str>,
    drop: Option<&str>,
) -> nir::program::Package {
    let path = std::env::temp_dir().join(format!(
        "luna-resources-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let artifact = frontend::compile_in_package_with_source_protocols(
        &Options {
            input: path.clone(),
            emit: Emit::Nir,
            output: None,
            sysroot: None,
            splice_protocol: None,
            meta_list_type: None,
            index_protocol: None,
            ord_protocol: None,
            eq_protocol: None,
            list_pattern_protocol: None,
            literal_into_protocol: None,
            arithmetic_protocols: Default::default(),
            tuple_type: None,
        },
        identity,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy,
            drop,
            ..Default::default()
        },
    );
    std::fs::remove_file(path).unwrap();
    let Artifact::Nir(package) = artifact.unwrap_or_else(|error| panic!("{source}: {error:?}"))
    else {
        panic!("expected NIR")
    };
    package
}
#[test]
fn source_moves_follow_delivery_order_and_branch_availability() {
    use elaboration::verification::moves::{Delivery, analyze};
    let cases = [
        ("fn run(value:Ticket) {take(value); take(value);}", false),
        ("fn run(value:Ticket) {let next=value; take(next);}", true),
        ("fn run(value:Ticket) {let next=value; take(value);}", false),
        (
            "fn run(value:Ticket, flag:bool) {if flag {take(value);} take(value);}",
            false,
        ),
        (
            "fn run(value:Ticket, flag:bool) {if flag {take(value);} else {take(value);}}",
            true,
        ),
        (
            "fn run(value:Ticket, flag:bool) {if flag {take(value); return;} take(value);}",
            true,
        ),
        (
            "fn run(value:Ticket) {if false {take(value);} take(value);}",
            true,
        ),
        (
            "fn run(value:Ticket) {take(value); return; take(value);}",
            true,
        ),
        ("fn run(value:Ticket) {pair(value, value);}", false),
    ];
    for (body, accepted) in cases {
        let package = compile(&format!(
            "struct Ticket {{}} fn take(value:Ticket) {{}} fn pair(first:Ticket, second:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze(
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        );
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn source_moves_report_cleanup_and_fail_without_copy_evidence() {
    use elaboration::verification::{
        moves::{Delivery, analyze},
        ownership::Availability,
    };
    let package = compile(
        "struct Ticket {} fn take(value:Ticket) {} fn run(value:Ticket, flag:bool) {if flag {take(value);}} fn twice(value:bool)->bool {value and value}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    let report = analyze(function, policy, 10000).unwrap();
    assert!(
        report
            .cleanups
            .iter()
            .any(|c| c.availability == Availability::Conditional)
    );
    assert!(report.to_string().starts_with("(move-report (transfers"));
    assert!(
        analyze(function, policy, 0)
            .unwrap_err()
            .to_string()
            .contains("Budget")
    );
    let twice = package
        .functions
        .iter()
        .find(|f| f.name == "twice")
        .unwrap();
    assert!(
        analyze(twice, |_| Err("copy legality is pending".into()), 10000)
            .unwrap_err()
            .reason
            .contains("pending")
    );
    assert!(analyze(twice, |_| Ok(Delivery::Copy), 10000).is_ok());
}

#[test]
fn move_assignments_restore_availability_and_retain_cleanup_responsibilities() {
    use elaboration::verification::moves::{Delivery, analyze};
    let package = compile(
        "struct Ticket {} fn take(value:Ticket) {} fn refill(value:Ticket) {var local=value; take(local); local=Ticket {}; take(local);} fn replace(value:Ticket) {var local=value; local=Ticket {}; take(local);} fn self_move(value:Ticket) {var local=value; local=local; take(local);} fn unused() {Ticket {};}",
    );
    let analyze_named = |name: &str| {
        analyze(
            package.functions.iter().find(|f| f.name == name).unwrap(),
            |_| Ok(Delivery::Move),
            10000,
        )
        .unwrap()
    };
    assert!(analyze_named("refill").cleanups.is_empty());
    assert_eq!(analyze_named("replace").cleanups.len(), 1);
    assert!(analyze_named("self_move").cleanups.is_empty());
    let unused = analyze_named("unused");
    assert_eq!(unused.temporaries.len(), 1);
    assert!(matches!(
        unused.temporaries[0].ty,
        nir::Term::Nominal { .. }
    ));
}

#[test]
fn incomplete_move_semantics_are_explicit_analysis_failures() {
    use elaboration::verification::moves::{Delivery, analyze};
    let package = compile(
        "struct Ticket {flag:bool} fn projected(value:Ticket)->bool {value.flag} fn defaulted(.value:bool = true)->bool {value} fn run()->bool {defaulted()}",
    );
    for name in ["projected"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let error = analyze(function, |_| Ok(Delivery::Copy), 10000).unwrap_err();
        assert!(
            error.reason.contains("semantics") || error.reason.contains("parameter frame"),
            "{error}"
        );
    }
}

#[test]
fn source_field_moves_preserve_siblings_and_reject_whole_partial_values() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run(value:Pair) {take(value.left); take(value.right);}",
            true,
        ),
        (
            "fn run(value:Pair) {take(value.left); take(value.left);}",
            false,
        ),
        ("fn run(value:Pair)->Pair {take(value.left); value}", false),
        (
            "fn run(value:Pair) {whole(value); take(value.right);}",
            false,
        ),
        (
            "fn run(value:Pair, flag:bool) {if flag {take(value.left);} take(value.right);}",
            true,
        ),
        (
            "fn run(value:Pair, flag:bool) {if flag {take(value.left);} take(value.left);}",
            false,
        ),
        (
            "fn run(value:Pair)->Pair {var p=value; take(p.left); p.left=Ticket {}; p}",
            true,
        ),
        (
            "fn run(value:Holder) {take(value.inner.left); take(value.inner.right);}",
            true,
        ),
        (
            "fn run(value:Wrapper<Ticket>)->bool {let a=value.flag; let b=value.flag; take(value.item); a and b}",
            true,
        ),
    ];
    for (body, accepted) in cases {
        let package = compile(&format!(
            "struct Ticket {{}} struct Pair {{left:Ticket,right:Ticket}} struct Holder {{inner:Pair}} struct Wrapper where T {{item:T,flag:bool}} fn take(value:Ticket) {{}} fn whole(value:Pair) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        );
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn field_replacement_records_only_overwritten_responsibilities() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} fn take(value:Ticket) {} fn run(value:Pair) {var p=value; p.left=Ticket {}; take(p.left); take(p.right);}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
    let replacement = report
        .cleanup_sites
        .iter()
        .find(|site| site.reason == elaboration::verification::moves::CleanupReason::Replacement)
        .unwrap();
    let replaced: Vec<_> = report
        .cleanups
        .iter()
        .filter(|cleanup| cleanup.source == replacement.source)
        .collect();
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].place.fields, vec![0]);
    // With no Drop policy, calls retain separate possible failure obligations.
    assert!(report.cleanup_sites.iter().any(
        |site| site.reason == elaboration::verification::moves::CleanupReason::PredicateFailure
    ));
}

#[test]
fn destructuring_transfers_each_owned_component_once() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run(value:Tuple<Ticket,Ticket>) {let (left,right)=value; take(left); take(right);}",
            true,
        ),
        (
            "fn run(value:Tuple<Ticket,Ticket>) {let (left,right)=value; take(left); take(left);}",
            false,
        ),
        (
            "fn run(value:Tuple<Ticket,Ticket>) {let (left,right)=value; let again=value;}",
            false,
        ),
        (
            "fn run(value:Pair) {let {left,right}=value; take(left); take(right);}",
            true,
        ),
        (
            "fn run(value:Pair) {let {left}=value; take(left); take(value.right);}",
            false,
        ),
        (
            "fn run(value:Wrapper<Tuple<Ticket,Ticket>>) {let {item:(left,right)}=value; take(left); take(right);}",
            true,
        ),
    ];
    for (body, accepted) in cases {
        let package = tuple_support::compile_with_tuple(&format!(
            "struct Ticket {{}} struct Pair {{left:Ticket,right:Ticket}} struct Wrapper where T {{item:T}} fn take(value:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000);
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn wildcard_destructuring_retains_exact_residual_cleanup_paths() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = tuple_support::compile_with_tuple(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} fn take(value:Ticket) {} fn tuple(value:Tuple<Tuple<Ticket,Ticket>,Ticket>) {let ((_,picked),_)=value; take(picked);} fn record(value:Pair) {let {left}=value; take(left);}",
    );
    for (name, paths) in [
        ("tuple", vec![vec![0, 0], vec![1]]),
        ("record", vec![vec![1]]),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
        assert!(report.cleanups.is_empty(), "{report}");
        assert_eq!(
            report
                .temporaries
                .iter()
                .map(|t| t.fields.clone())
                .collect::<Vec<_>>(),
            paths
        );
        assert!(
            report
                .temporaries
                .iter()
                .all(|t| matches!(t.ty, nir::Term::Nominal { .. }))
        );
        let repeated =
            analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
        assert_eq!(report.to_string(), repeated.to_string());
    }
}

#[test]
fn overlapping_pattern_moves_are_not_silently_accepted() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = tuple_support::compile_with_tuple(
        "struct Ticket {} fn overlap(value:Tuple<Ticket,Ticket>) {let (left,right) as whole=value;} fn refutable(value:bool) {let true=value else {return;};}",
    );
    for (name, message) in [("overlap", "overlapping")] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let error =
            analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap_err();
        assert!(error.reason.contains(message), "{name}: {error}");
    }
}

#[test]
fn closure_construction_moves_captures_without_executing_body_transfers() {
    use elaboration::verification::{
        moves::{Delivery, analyze_in_package},
        ownership::Place,
    };
    let package = compile(
        "struct Ticket {} fn take(value:Ticket) {} fn own(value:Ticket) {let callback=|| take(value);} fn read(value:bool) {let callback=|| value;} fn nested(value:Ticket) {let callback=|| {let inner=|| take(value);};}",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    for (name, consumed) in [("own", true), ("read", false)] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, policy, 10000).unwrap();
        assert_eq!(report.closures.len(), 1);
        let closure = &report.closures[0];
        assert_eq!(closure.captures.len(), 1);
        assert!(
            closure
                .invocation
                .cleanups
                .iter()
                .all(|c| !closure.captures.contains(&c.place.root))
        );
        let captured = Place {
            root: closure.captures[0],
            fields: vec![],
        };
        assert_eq!(
            closure.invocation.exits[0]
                .require_present(&captured)
                .is_err(),
            consumed
        );
        // The never-invoked closure still owns its constructed environment.
        assert!(report.cleanups.iter().any(|c| c.place.fields == vec![0]));
        assert!(report.to_string().contains("(invocation (move-report"));
    }
    let nested = package
        .functions
        .iter()
        .find(|f| f.name == "nested")
        .unwrap();
    let report = analyze_in_package(&package, nested, policy, 10000).unwrap();
    assert_eq!(report.closures[0].invocation.closures.len(), 1);
}

#[test]
fn closure_capture_and_body_reject_duplicate_responsibility() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for body in [
        "fn run(value:Ticket) {let callback=|| take(value); take(value);}",
        "fn run(value:Ticket) {let callback=|| {take(value); take(value);};}",
        "fn run(value:Ticket) {take(value); let callback=|| take(value);}",
    ] {
        let package = compile(&format!(
            "struct Ticket {{}} fn take(value:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let error =
            analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap_err();
        assert!(error.reason.contains("unavailable"), "{body}: {error}");
    }
}

#[test]
fn closure_call_is_not_misclassified_as_whole_environment_move() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile("fn run(value:bool) {let read=|| value; read(); read();}");
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 10000).unwrap();
    assert_eq!(report.closures.len(), 3);
    assert!(
        report
            .closures
            .iter()
            .all(|c| c.invocation.cleanups.is_empty())
    );
}

#[test]
fn closure_invocations_transfer_current_environment_state() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run(value:Ticket) {let action=|| take(value); action();}",
            true,
        ),
        (
            "fn run(value:Ticket) {let action=|| take(value); action(); action();}",
            false,
        ),
        (
            "fn run(value:Ticket, flag:bool) {let action=|| take(value); if flag {action();} action();}",
            false,
        ),
        (
            "fn run(value:Ticket, flag:bool) {let action=|| take(value); if flag {action();} else {action();}}",
            true,
        ),
        (
            "fn run(value:Ticket) {let action=|| take(value); let transferred=action; transferred();}",
            true,
        ),
        (
            "fn run(value:Ticket) {let action=|| take(value); let transferred=action; action();}",
            false,
        ),
        (
            "fn run(value:Ticket) {let action=|first:Ticket, second:Ticket| {take(first); take(second);}; action(value,value);}",
            false,
        ),
    ];
    for (body, accepted) in cases {
        let package = compile(&format!(
            "struct Ticket {{}} fn take(value:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        );
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        if let Ok(report) = result {
            assert!(
                report.cleanups.iter().all(|c| c.place.fields.is_empty()),
                "captured Ticket was cleaned after transfer: {body}: {report}"
            );
        }
    }
}

#[test]
fn closure_receiver_replacement_during_arguments_is_not_confused_with_call_target() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "fn run(flag:bool) {var action=|value:bool| flag; let saved=action; action(do {action=saved; true});}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    // Supply representation-copy permission explicitly to exercise two stored
    // environments with identical body provenance; no builtin inference here.
    let error = analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 10000).unwrap_err();
    assert!(error.reason.contains("receiver changed"), "{error}");
}

#[test]
fn loop_move_fixed_points_check_backedges_and_reinitialization() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run(value:Ticket, flag:bool) {while flag {take(value);}}",
            false,
        ),
        (
            "fn run(value:Ticket) {while true {take(value); break;}}",
            true,
        ),
        (
            "fn run(value:Ticket) {while true {take(value); continue;}}",
            false,
        ),
        (
            "fn run(value:Ticket, flag:bool) {var local=value; while flag {take(local); local=Ticket {};}}",
            true,
        ),
        (
            "fn run(value:Ticket, flag:bool) {var local=value; while flag {take(local); continue; local=Ticket {};}}",
            false,
        ),
        (
            "fn run(flag:bool) {while flag {let local=Ticket {}; take(local);}}",
            true,
        ),
        (
            "fn run(value:Ticket) {while false {take(value);} take(value);}",
            true,
        ),
        (
            "fn run(value:Ticket, flag:bool) {while flag {take(value); break;} take(value);}",
            false,
        ),
        (
            "fn run(value:Ticket) {while true {while true {take(value); break;} break;}}",
            true,
        ),
        (
            "fn run(value:Ticket) {while:outer true {while true {take(value); break outer;}}}",
            true,
        ),
        (
            "fn run(value:Ticket) {while:outer true {while true {take(value); continue outer;}}}",
            false,
        ),
        (
            "fn run(value:Ticket) {while true {take(value); return;} take(value);}",
            true,
        ),
    ];
    for (body, accepted) in cases {
        let package = compile(&format!(
            "struct Ticket {{}} fn take(value:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        );
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{body}: {error}");
        }
    }
}

#[test]
fn loop_exits_record_local_cleanup_once_and_preserve_closure_environment_rules() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn take(value:Ticket) {} fn break_cleanup() {while true {let local=Ticket {}; break;}} fn continue_cleanup(flag:bool) {while flag {let local=Ticket {}; continue;}} fn callable(value:Ticket, flag:bool) {let action=|| take(value); while flag {action();}} fn labeled_cleanup() {while:outer true {let first=Ticket {}; while true {let second=Ticket {}; break outer;}}} fn observer(value:bool, flag:bool) {let read=|| value; while flag {read();}}",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    let run = |name: &str| {
        analyze_in_package(
            &package,
            package.functions.iter().find(|f| f.name == name).unwrap(),
            policy,
            10000,
        )
    };
    assert_eq!(run("labeled_cleanup").unwrap().cleanups.len(), 2);
    let before = package.to_string();
    let observer = package
        .functions
        .iter()
        .find(|f| f.name == "observer")
        .unwrap();
    assert!(analyze_in_package(&package, observer, policy, 5).is_err());
    let report = run("break_cleanup").unwrap();
    assert_eq!(report.cleanups.len(), 1);
    assert_eq!(report.loops.len(), 1);
    assert_eq!(report.loops[0].breaks.len(), 1);
    assert!(report.to_string().contains("(loops (loop"));
    // One cleanup site in the loop plus one retained flag responsibility at exit.
    assert_eq!(run("continue_cleanup").unwrap().cleanups.len(), 2);
    assert!(run("callable").unwrap_err().reason.contains("unavailable"));
    assert!(run("observer").is_ok());
    assert_eq!(package.to_string(), before);
}

#[test]
fn scalar_defaults_use_parameter_frames_in_source_and_declaration_order() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for body in [
        "fn target(value:bool, .first:bool=value, .second:bool=not first)->bool {second} fn run(value:bool)->bool {target(value)}",
        "fn target(value:bool, .first:bool=value, .second:bool=not first)->bool {second} fn run(value:bool)->bool {target(.first=value, false)}",
        "fn target(value:bool, .first:bool=value, .second:bool=not first)->bool {second} fn run(value:bool)->bool {target(.second=value, false)}",
    ] {
        let package = compile(body);
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 10000).unwrap();
        assert_eq!(report.calls.len(), 1);
        assert_eq!(report.calls[0].parameters.remaining().len(), 3);
    }
}

#[test]
fn normalized_default_frame_rejects_consumption_of_another_argument() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::{Expr, ExprKind as K};
    // Source P3 default checking currently rejects nominal-value defaults before
    // ownership analysis. Build the normalized preparation plan directly to
    // test this independent resource boundary without weakening that check.
    let package = compile(
        "struct Ticket {} fn target(value:Ticket, other:Ticket) {} fn run(value:Ticket) {target(value, Ticket {});}",
    );
    let target = package
        .functions
        .iter()
        .find(|f| f.name == "target")
        .unwrap();
    let mut function = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .clone();
    fn call(expr: &mut Expr) -> &mut Expr {
        if matches!(expr.kind, K::Call { .. }) {
            return expr;
        }
        match &mut expr.kind {
            K::Block { statements, .. } => call(&mut statements[0]),
            K::Discard(value) => call(value),
            _ => panic!("call expected"),
        }
    }
    let K::Call {
        arguments,
        parameter_bindings,
        ..
    } = &mut call(&mut function.body).kind
    else {
        unreachable!()
    };
    *parameter_bindings = target.parameters.clone();
    arguments[1].is_default = true;
    arguments[1].value.kind = K::Local(target.parameters[0]);
    let error = analyze_in_package(&package, &function, |_| Ok(Delivery::Move), 10000).unwrap_err();
    assert!(error.reason.contains("consumed a parameter"), "{error}");
    // Broken declaration order cannot be accepted either.
    let K::Call { arguments, .. } = &mut call(&mut function.body).kind else {
        unreachable!()
    };
    arguments.swap(0, 1);
    let error = analyze_in_package(&package, &function, |_| Ok(Delivery::Move), 10000).unwrap_err();
    assert!(
        error.reason.contains("explicit argument follows"),
        "{error}"
    );
}

#[test]
fn default_frames_do_not_alias_recursive_caller_locals_or_schedule_parameter_cleanup() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn recursive(value:bool, .other:bool=value)->bool {if value {recursive(false)} else {other}} fn target(value:Ticket, .flag:bool=true) {} fn run(value:Ticket) {target(value);}",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, run, policy, 10000).unwrap();
    assert_eq!(report.calls.len(), 1);
    assert!(report.cleanups.is_empty(), "{report}");
    assert_eq!(report.calls[0].parameters.remaining().len(), 2);
    let recursive = package
        .functions
        .iter()
        .find(|f| f.name == "recursive")
        .unwrap();
    let report = analyze_in_package(&package, recursive, policy, 10000).unwrap();
    assert_eq!(report.calls.len(), 1);
    assert!(report.to_string().contains("(call-preparations (call"));
}

#[test]
fn match_arms_transfer_owned_payloads_and_join_only_live_paths() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run(value:Maybe<Ticket>) {value match {.none=>(), .some(item)=>take(item)}}",
            true,
        ),
        (
            "fn run(value:Maybe<Ticket>) {value match {.none=>(), .some(item)=>{take(item); take(item);}}}",
            false,
        ),
        (
            "fn run(value:Ticket, flag:bool) {flag match {true=>take(value),false=>take(value)}}",
            true,
        ),
        (
            "fn run(value:Ticket, flag:bool) {flag match {true=>take(value),false=>()}; take(value);}",
            false,
        ),
        (
            "fn run(value:Ticket, flag:bool) {flag match {true=>{take(value); return;},false=>()}; take(value);}",
            true,
        ),
        (
            "fn run(value:Ticket) {false match {true=>take(value),false=>()}; take(value);}",
            true,
        ),
        (
            "fn run(value:Tuple<Ticket,bool>) {value match {(item,true)=>take(item),(item,false)=>take(item)}}",
            true,
        ),
        (
            "fn run(value:Maybe<Ticket>) {value match {.none=>(),.some(item)=>take(item)}; let again=value;}",
            false,
        ),
    ];
    for (body, accepted) in cases {
        let package = tuple_support::compile_with_tuple(&format!(
            "struct Ticket {{}} enum Maybe where T {{none,some(T)}} fn take(value:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        );
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{body}: {error}");
        }
    }
}

#[test]
fn match_cleanup_records_variant_residuals_and_empty_owned_decomposition() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} fn take(value:Ticket) {} fn run(value:Choice) {value match {.empty=>(),.pair(item,_)=>take(item)}} fn empty(value:Ticket) {let {}=value;}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, run, |_| Ok(Delivery::Move), 10000).unwrap();
    assert_eq!(report.matches.len(), 1);
    assert_eq!(report.matches[0].arms.len(), 2);
    assert_eq!(
        report
            .decompositions
            .iter()
            .map(|d| d.variant)
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1)]
    );
    assert_eq!(
        report
            .temporaries
            .iter()
            .map(|t| t.fields.clone())
            .collect::<Vec<_>>(),
        vec![vec![], vec![1]]
    );
    let empty = package
        .functions
        .iter()
        .find(|f| f.name == "empty")
        .unwrap();
    let report = analyze_in_package(&package, empty, |_| Ok(Delivery::Move), 10000).unwrap();
    assert_eq!(report.decompositions.len(), 1);
    assert_eq!(report.temporaries.len(), 1);
    assert!(report.temporaries[0].fields.is_empty());
}

#[test]
fn match_control_transfers_compose_with_loops_and_closure_invocations() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} enum Maybe where T {none,some(T)} fn take(value:Ticket) {} fn exit_loop(value:Ticket,flag:bool) {while true {flag match {true=>{take(value); break;},false=>continue}}} fn bad_loop(value:Ticket,flag:bool) {while true {flag match {true=>{take(value); continue;},false=>break}}} fn repeated(value:Maybe<Ticket>) {let action=|| value match {.none=>(),.some(item)=>take(item)}; action(); action();} fn selected() {Maybe<Ticket>.some(Ticket {}) match {.none=>(),.some(item)=>take(item)}} fn guarded(value:Maybe<Ticket>, flag:bool) {value match {.some(item) if flag=>take(item),_=>()}}",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    let run = |name: &str| {
        analyze_in_package(
            &package,
            package.functions.iter().find(|f| f.name == name).unwrap(),
            policy,
            10000,
        )
    };
    assert!(run("exit_loop").is_ok());
    assert!(run("bad_loop").unwrap_err().reason.contains("unavailable"));
    assert!(run("repeated").unwrap_err().reason.contains("unavailable"));
    let selected = run("selected").unwrap();
    assert_eq!(selected.matches[0].arms.len(), 1);
    assert_eq!(selected.decompositions[0].variant, Some(1));
    assert!(run("guarded").is_ok());
}

#[test]
fn read_only_guards_keep_payload_available_for_later_candidates() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        "fn run(value:Maybe<Ticket>,flag:bool) {value match {.some(item) if flag=>take(item),.some(item)=>take(item),.none=>()}}",
        "fn run(value:Maybe<Ticket>) {value match {.some(item) if item.flag=>take(item),.some(item)=>take(item),.none=>()}}",
        "fn run(value:Maybe<Ticket>) {value match {.some(item) if false=>{take(item); take(item);},.some(item)=>take(item),.none=>()}}",
        "fn run(value:Maybe<Ticket>) {value match {.some(item) if false and consume(item)=>(),.some(item)=>take(item),.none=>()}}",
    ];
    for body in cases {
        let package = compile(&format!(
            "struct Ticket {{flag:bool}} enum Maybe where T {{none,some(T)}} fn take(value:Ticket) {{}} fn consume(value:Ticket)->bool {{true}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        )
        .unwrap();
        assert!(
            report.cleanups.iter().all(|c| c.place.fields.is_empty()),
            "{report}"
        );
        if body.contains("if false") {
            assert_eq!(report.matches[0].arms.len(), 2);
            assert_eq!(report.decompositions.len(), 2);
        } else {
            assert_eq!(report.matches[0].arms.len(), 3);
            assert_eq!(report.matches[0].arms[0].guards.len(), 1);
        }
    }
}

#[test]
fn guard_observation_checks_availability_and_rejects_unmodeled_effects() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let sources = [
        (
            "fn run(value:Ticket,flag:bool) {take(value); flag match {true if value.flag=>(),_=>()}}",
            "unavailable",
        ),
        (
            "fn run(value:Maybe<Ticket>) {value match {.some(item) if consume(item)=>(),_=>()}}",
            "observation cleanup requires captured selection evidence",
        ),
    ];
    for (body, message) in sources {
        let package = compile(&format!(
            "struct Ticket {{flag:bool}} enum Maybe where T {{none,some(T)}} fn take(value:Ticket) {{}} fn consume(value:Ticket)->bool {{true}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let error = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        )
        .unwrap_err();
        assert!(error.reason.contains(message), "{body}: {error}");
    }
}

#[test]
fn let_else_transfers_success_bindings_and_excludes_diverging_failure_paths() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run(value:Maybe<Ticket>) {let .some(item)=value else {return;}; take(item);}",
            true,
        ),
        (
            "fn run(value:Maybe<Ticket>) {let .some(item)=value else {return;}; take(item); take(item);}",
            false,
        ),
        (
            "fn run(value:Maybe<Ticket>, other:Ticket) {let .some(item)=value else {take(other); return;}; take(other); take(item);}",
            true,
        ),
        (
            "fn run(value:Maybe<Ticket>,flag:bool) {let (.some(item) if flag)=value else {return;}; take(item);}",
            true,
        ),
        (
            "fn run(value:Maybe<Ticket>,flag:bool) {while flag {let .some(item)=value else {break;}; take(item); break;}}",
            true,
        ),
        (
            "fn run(value:Maybe<Ticket>,flag:bool) {while flag {let .some(item)=value else {continue;}; take(item); break;}}",
            false,
        ),
        (
            "fn run(flag:bool) {var value=Maybe<Ticket>.none; while flag {let .some(item)=value else {value=Maybe<Ticket>.some(Ticket {}); continue;}; take(item); break;}}",
            true,
        ),
    ];
    for (body, accepted) in cases {
        let package = compile(&format!(
            "struct Ticket {{}} enum Maybe where T {{none,some(T)}} fn take(value:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        );
        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{body}: {error}");
        }
    }
}

#[test]
fn let_else_failure_keeps_subject_cleanup_without_committing_payload_bindings() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} enum Maybe where T {none,some(T)} fn take(value:Ticket) {} fn unknown(value:Maybe<Ticket>) {let .some(item)=value else {return;}; take(item);} fn failed() {let .some(item)=Maybe<Ticket>.none else {return;}; take(item); take(item);} fn selected(other:Ticket) {let .some(item)=Maybe<Ticket>.some(Ticket {}) else {take(other); return;}; take(other); take(item);}",
    );
    let run = |name: &str| {
        analyze_in_package(
            &package,
            package.functions.iter().find(|f| f.name == name).unwrap(),
            |_| Ok(Delivery::Move),
            10000,
        )
        .unwrap()
    };
    let unknown = run("unknown");
    assert!(unknown.bindings[0].success && unknown.bindings[0].failure);
    assert_eq!(unknown.temporaries.len(), 1);
    assert!(unknown.temporaries[0].fields.is_empty());
    let failed = run("failed");
    assert!(!failed.bindings[0].success && failed.bindings[0].failure);
    assert!(failed.decompositions.is_empty());
    assert_eq!(failed.temporaries.len(), 1);
    let selected = run("selected");
    assert!(selected.bindings[0].success && !selected.bindings[0].failure);
    assert!(selected.temporaries.is_empty());
    assert!(selected.to_string().contains("(pattern-bindings (binding"));
}

#[test]
fn normalized_let_else_cannot_continue_from_a_failed_binding() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::ExprKind as K;
    let package = compile("fn run(value:bool) {let true=value else {return;};}");
    let mut function = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .clone();
    let K::Block { statements, .. } = &mut function.body.kind else {
        panic!("block")
    };
    let K::LetPattern {
        otherwise: Some(otherwise),
        ..
    } = &mut statements[0].kind
    else {
        panic!("let pattern")
    };
    otherwise.kind = K::Unit;
    otherwise.ty = nir::Term::Primitive(nir::Primitive::Void);
    let error = analyze_in_package(&package, &function, |_| Ok(Delivery::Copy), 10000).unwrap_err();
    assert!(
        error.reason.contains("does not establish divergence"),
        "{error}"
    );
    let original = package.functions.iter().find(|f| f.name == "run").unwrap();
    assert!(analyze_in_package(&package, original, |_| Ok(Delivery::Copy), 10000).is_ok());
}

#[test]
fn wildcard_ignores_existing_positions_without_transferring_their_values() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for body in [
        "fn run(value:Ticket) {let _=value; take(value);}",
        "fn run(value:Ticket) {take(value); let _=value;}",
        "fn run(value:Pair) {let _=value.left; take(value.left); take(value.right);}",
        "fn run(value:Pair) {take(value.left); let _=value.left; take(value.right);}",
        "fn run(value:Ticket,flag:bool) {let (_ if flag)=value else {take(value);return;}; take(value);}",
    ] {
        let package = compile(&format!(
            "struct Ticket {{}} struct Pair {{left:Ticket,right:Ticket}} fn take(value:Ticket) {{}} {body}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        )
        .unwrap();
        assert!(report.temporaries.is_empty(), "{body}: {report}");
    }
}

#[test]
fn wildcard_created_values_still_evaluate_inputs_and_keep_cleanup() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn pass(value:Ticket)->Ticket {value} fn take(value:Ticket) {} fn run(value:Ticket) {let _=pass(value);} fn bad(value:Ticket) {let _=pass(value);take(value);} fn constructed() {let _=Ticket {};}",
    );
    for name in ["run", "constructed"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
        assert_eq!(report.temporaries.len(), 1);
        assert!(report.cleanups.is_empty());
    }
    let function = package.functions.iter().find(|f| f.name == "bad").unwrap();
    assert!(
        analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000)
            .unwrap_err()
            .reason
            .contains("unavailable")
    );
}

#[test]
fn cleanup_obligations_follow_declaration_order_not_local_id_sorting() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::ExprKind as K;
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} fn run() {let first=Ticket {};let second=Pair {left:Ticket {},right:Ticket {}};}",
    );
    for (first_id, second_id) in [(9000, 1), (1, 9000)] {
        let mut function = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .clone();
        let K::Block { statements, .. } = &mut function.body.kind else {
            panic!("block")
        };
        let K::Let { binding: first, .. } = &mut statements[0].kind else {
            panic!("first")
        };
        first.index = first_id;
        let K::Let {
            binding: second, ..
        } = &mut statements[1].kind
        else {
            panic!("second")
        };
        second.index = second_id;
        let report =
            analyze_in_package(&package, &function, |_| Ok(Delivery::Move), 10000).unwrap();
        assert_eq!(
            report
                .cleanups
                .iter()
                .map(|c| (c.place.root.local, c.place.fields.clone()))
                .collect::<Vec<_>>(),
            vec![
                (second_id, vec![1]),
                (second_id, vec![0]),
                (first_id, vec![])
            ]
        );
    }
}

#[test]
fn assignment_return_and_closure_cleanup_preserve_semantic_order() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn take(value:Ticket) {} fn assigned() {var first=Ticket {};let second=Ticket {};first=Ticket {};} fn returned() {let first=Ticket {};let second=Ticket {};return;} fn captured(first:Ticket,second:Ticket) {let action=|| {take(second);take(first);};}",
    );
    let run = |name: &str| {
        analyze_in_package(
            &package,
            package.functions.iter().find(|f| f.name == name).unwrap(),
            |_| Ok(Delivery::Move),
            10000,
        )
        .unwrap()
    };
    let assigned = run("assigned");
    assert_eq!(assigned.cleanups.len(), 3);
    assert_eq!(
        assigned.cleanups[0].place.root,
        assigned.cleanups[2].place.root
    );
    assert_ne!(
        assigned.cleanups[0].place.root,
        assigned.cleanups[1].place.root
    );
    let returned = run("returned");
    assert_eq!(returned.cleanups.len(), 2);
    let captured = run("captured");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "captured")
        .unwrap();
    assert_eq!(
        captured.closures[0]
            .captures
            .iter()
            .map(|p| p.local)
            .collect::<Vec<_>>(),
        vec![function.parameters[1].index, function.parameters[0].index]
    );
    assert_eq!(
        captured
            .cleanups
            .iter()
            .map(|c| c.place.fields.clone())
            .collect::<Vec<_>>(),
        vec![vec![1], vec![0]]
    );
}

#[test]
fn temporary_projection_evaluates_base_once_and_preserves_residual_fields() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} struct Outer {inner:Pair,last:Ticket} fn take(value:Ticket) {} fn pair(value:Ticket)->Pair {Pair {left:value,right:Ticket {}}} fn outer(value:Ticket)->Outer {Outer {inner:pair(value),last:Ticket {}}} fn run(value:Ticket) {let selected=pair(value).left;take(selected);} fn nested(value:Ticket) {let selected=outer(value).inner.left;take(selected);} fn invalid(value:Ticket) {let selected=pair(value).left;take(value);}",
    );
    for (name, paths) in [
        ("run", vec![vec![1]]),
        ("nested", vec![vec![1], vec![0, 1]]),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
        assert_eq!(report.temporary_owners.len(), 1);
        assert_eq!(
            report
                .temporaries
                .iter()
                .map(|t| t.fields.clone())
                .collect::<Vec<_>>(),
            paths
        );
        assert!(report.cleanups.is_empty());
        assert_eq!(
            report.temporary_owners[0].remaining.remaining().len(),
            paths.len()
        );
        assert!(report.to_string().contains("(temporary-owners (owner"));
    }
    let invalid = package
        .functions
        .iter()
        .find(|f| f.name == "invalid")
        .unwrap();
    assert!(
        analyze_in_package(&package, invalid, |_| Ok(Delivery::Move), 10000)
            .unwrap_err()
            .reason
            .contains("unavailable")
    );
}

#[test]
fn copy_projection_keeps_parent_field_and_discarded_move_keeps_both_responsibilities() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} struct Tagged {flag:bool,owned:Ticket} fn tagged()->Tagged {Tagged {flag:true,owned:Ticket {}}} fn pair()->Pair {Pair {left:Ticket {},right:Ticket {}}} fn copied()->bool {tagged().flag} fn ignored() {let _=pair().left;}",
    );
    let copied = package
        .functions
        .iter()
        .find(|f| f.name == "copied")
        .unwrap();
    let report = analyze_in_package(
        &package,
        copied,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        10000,
    )
    .unwrap();
    assert_eq!(
        report
            .temporaries
            .iter()
            .map(|t| t.fields.clone())
            .collect::<Vec<_>>(),
        vec![vec![1], vec![0]]
    );
    assert_eq!(report.temporary_owners[0].remaining.remaining().len(), 2);
    let ignored = package
        .functions
        .iter()
        .find(|f| f.name == "ignored")
        .unwrap();
    let report = analyze_in_package(&package, ignored, |_| Ok(Delivery::Move), 10000).unwrap();
    assert_eq!(report.temporary_owners[0].remaining.remaining().len(), 1);
    assert_eq!(report.temporaries.len(), 2);
    assert!(report.temporaries.iter().any(|t| t.fields.is_empty()));
}

#[test]
fn generic_temporary_projection_retains_parent_even_after_last_field_moves() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} struct Wrapper where T {item:T} fn wrap(value:T)->Wrapper<T> where T {Wrapper<T> {item:value}} fn run(value:Ticket)->Ticket {wrap(value).item}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
    assert_eq!(report.temporary_owners.len(), 1);
    assert!(report.temporary_owners[0].remaining.remaining().is_empty());
    assert!(report.temporaries.is_empty());
    assert!(report.cleanups.is_empty());
    let nir::Term::Nominal { arguments, .. } = &report.temporary_owners[0].ty else {
        panic!("parent type")
    };
    assert_eq!(arguments.len(), 1);
    assert!(matches!(arguments[0], nir::Term::Nominal { .. }));
}

#[test]
fn temporary_reports_distinguish_expression_condition_match_and_return_boundaries() {
    use elaboration::verification::moves::{Delivery, TemporaryBoundary as B, analyze_in_package};
    let package = compile(
        "struct Ticket {} struct Tagged {flag:bool,owned:Ticket} fn make()->Tagged {Tagged {flag:true,owned:Ticket {}}} fn use_flag(value:bool) {} fn run() {let picked=make().flag; if make().flag {let _=make();} make() match {{flag:bit}=>use_flag(bit)};} fn returned()->bool {return make().flag;} fn tail()->bool {make().flag} fn nested() {let picked=do {let _=make();make().flag};}",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    let run = |name: &str| {
        analyze_in_package(
            &package,
            package.functions.iter().find(|f| f.name == name).unwrap(),
            policy,
            10000,
        )
        .unwrap()
    };
    let report = run("run");
    assert_eq!(
        report
            .temporary_owners
            .iter()
            .map(|t| t.scope.boundary)
            .collect::<Vec<_>>(),
        vec![B::FullExpression, B::Condition]
    );
    assert!(
        report
            .temporaries
            .iter()
            .any(|t| t.scope.boundary == B::Match)
    );
    assert!(
        report
            .temporaries
            .iter()
            .any(|t| t.scope.boundary == B::Condition)
    );
    let returned = run("returned");
    assert_eq!(returned.temporary_owners[0].scope.boundary, B::ReturnValue);
    let tail = run("tail");
    assert_eq!(tail.temporary_owners[0].scope.boundary, B::FunctionResult);
    let tail_function = package.functions.iter().find(|f| f.name == "tail").unwrap();
    assert_eq!(
        tail.temporary_owners[0].scope.source,
        tail_function.body.source
    );
    let nested = run("nested");
    assert_eq!(nested.temporary_owners[0].scope.boundary, B::FullExpression);
    assert_ne!(
        nested.temporaries[0].scope.source,
        nested.temporary_owners[0].scope.source
    );
    assert!(report.to_string().contains("(temporary-scope (kind match)"));
}

#[test]
fn closure_body_temporary_scopes_do_not_inherit_construction_scope() {
    use elaboration::verification::moves::{Delivery, TemporaryBoundary as B, analyze_in_package};
    let package = compile(
        "struct Tagged {flag:bool} fn make()->Tagged {Tagged {flag:true}} fn run() {let reader=|| make().flag;reader();}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 10000).unwrap();
    assert!(report.temporary_owners.is_empty());
    assert_eq!(report.closures.len(), 2);
    for closure in &report.closures {
        assert_eq!(
            closure.invocation.temporary_owners[0].scope.boundary,
            B::FunctionResult
        );
        assert_ne!(
            closure.invocation.temporary_owners[0].scope.source,
            function.body.source
        );
    }
}

#[test]
fn normalized_default_projection_has_its_own_temporary_boundary() {
    use elaboration::verification::moves::{Delivery, TemporaryBoundary as B, analyze_in_package};
    use nir::program::ExprKind as K;
    let package = compile(
        "struct Tagged {flag:bool} fn make()->Tagged {Tagged {flag:true}} fn field()->bool {make().flag} fn target(.flag:bool=true) {} fn run() {target();}",
    );
    let field = package
        .functions
        .iter()
        .find(|f| f.name == "field")
        .unwrap();
    let K::Block {
        result: Some(projection),
        ..
    } = &field.body.kind
    else {
        panic!("projection")
    };
    let mut run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .clone();
    let K::Block { statements, .. } = &mut run.body.kind else {
        panic!("block")
    };
    fn call(expr: &mut nir::program::Expr) -> &mut Vec<nir::program::CallArgument> {
        match &mut expr.kind {
            K::Call { arguments, .. } => arguments,
            K::Discard(value) => call(value),
            K::Block { statements, .. } => call(&mut statements[0]),
            other => panic!("call: {other:?}"),
        }
    }
    let arguments = call(&mut statements[0]);
    assert!(arguments[0].is_default);
    // The source P3 default checker does not yet accept this call/projection;
    // exercise the independently normalized ownership/lifetime boundary.
    arguments[0].value = projection.as_ref().clone();
    let report = analyze_in_package(
        &package,
        &run,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        10000,
    )
    .unwrap();
    assert_eq!(report.defaults.len(), 1);
    let report = &report.defaults[0].evaluation;
    assert_eq!(report.temporary_owners.len(), 1);
    assert_eq!(
        report.temporary_owners[0].scope.boundary,
        B::DefaultArgument
    );
    assert_eq!(report.temporary_owners[0].scope.source, projection.source);
}

#[test]
fn interrupted_preparations_keep_only_completed_components() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn pair(a:Ticket,b:Ticket) {} fn tuple_exit(value:Ticket) {let result=(value,do {return; Ticket {}});} fn call_exit(value:Ticket) {pair(value,do {return; Ticket {}});} fn conditional(value:Ticket,flag:bool) {pair(value,do {if flag {return;} Ticket {}});} fn successful(value:Ticket) {pair(value,Ticket {});}",
    );
    for name in ["tuple_exit", "call_exit", "conditional", "successful"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
        if name == "successful" {
            assert!(report.incomplete.is_empty(), "{report}");
            continue;
        }
        assert_eq!(report.incomplete.len(), 1, "{name}: {report}");
        let incomplete = &report.incomplete[0];
        assert_eq!(incomplete.values.len(), 1, "{report}");
        assert_eq!(incomplete.values[0].slot, 0);
        assert!(matches!(incomplete.values[0].ty, nir::Term::Nominal { .. }));
        let parameter = function.parameters[0];
        assert!(
            !report
                .cleanups
                .iter()
                .any(|c| c.place.root.owner == u64::from(parameter.owner)
                    && c.place.root.local == parameter.index),
            "moved argument cleaned twice: {report}"
        );
        assert!(
            report
                .to_string()
                .contains("(incomplete-cleanups (incomplete")
        );
    }
}

#[test]
fn nested_incomplete_construction_retains_reverse_order_and_lexical_anchors() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn run(a:Ticket,b:Ticket,c:Ticket) {let outer=Ticket {};let result=(a,do {let inner=Ticket {}; (b,c,do {return; Ticket {}})});}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
    assert_eq!(report.incomplete.len(), 2, "{report}");
    assert_eq!(
        report.incomplete[0]
            .values
            .iter()
            .map(|v| v.slot)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    assert_eq!(
        report.incomplete[1]
            .values
            .iter()
            .map(|v| v.slot)
            .collect::<Vec<_>>(),
        [0]
    );
    let inner = &report.incomplete[0].values[0].locals_before;
    let outer = &report.incomplete[1].values[0].locals_before;
    assert_eq!(inner.len(), outer.len() + 1, "{report}");
    assert!(inner.starts_with(outer));
    assert_eq!(
        report.cleanups.len(),
        2,
        "only the two local owners survive: {report}"
    );
}

#[test]
fn loop_jumps_retire_only_preparations_inside_the_target_loop() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn pair(a:Ticket,b:Ticket) {} fn preserved(value:Ticket) {pair(value,do {while true {break;} Ticket {}});} fn broken() {while true {pair(Ticket {},do {break;Ticket {}});}} fn continued(flag:bool) {while flag {pair(Ticket {},do {continue;Ticket {}});}} fn labeled() {while:outer true {pair(Ticket {},do {while true {pair(Ticket {},do {break outer;Ticket {}});} Ticket {}});}}",
    );
    for (name, expected) in [
        ("preserved", 0),
        ("broken", 1),
        ("continued", 1),
        ("labeled", 2),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            20000,
        )
        .unwrap();
        assert_eq!(report.incomplete.len(), expected, "{name}: {report}");
        assert!(
            report.incomplete.iter().all(|c| c.values.len() == 1),
            "{report}"
        );
    }
}

#[test]
fn closure_returns_do_not_retire_caller_preparations() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn run(value:Ticket) {let action=|| do {return Ticket {};};let result=(value,action());} fn latent(value:Ticket) {let result=(value,|| do {return Ticket {};});} fn interrupted(value:Ticket) {let action=|a:Ticket,b:Ticket| ();action(value,do {return;Ticket {}});}",
    );
    for name in ["run", "latent", "interrupted"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000).unwrap();
        assert_eq!(
            report.incomplete.len(),
            usize::from(name == "interrupted"),
            "{name}: {report}"
        );
        assert!(
            report
                .closures
                .iter()
                .all(|c| c.invocation.incomplete.is_empty()),
            "{report}"
        );
    }
}

#[test]
fn interrupted_explicit_arguments_in_default_calls_preserve_each_exit() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} fn target(first:Ticket,second:Ticket,.flag:bool=true) {} fn run(value:Ticket,flag:bool) {target(value,do {if flag {return;} return; Ticket {}});}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
    assert!(report.calls.is_empty(), "no complete call frame: {report}");
    assert_eq!(report.incomplete.len(), 2, "both return paths: {report}");
    for item in &report.incomplete {
        assert_eq!(item.values.len(), 1, "{report}");
        assert_eq!(item.values[0].slot, 0, "only the first argument completed");
    }
    assert_ne!(report.incomplete[0].exit, report.incomplete[1].exit);
}

#[test]
fn nominal_partial_construction_does_not_schedule_whole_object_cleanup() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} enum Choice {pair(Ticket,Ticket)} fn record(value:Ticket) {let result=Pair {left:value,right:do {return;Ticket {}}};} fn variant(value:Ticket) {let result=Choice.pair(value,do {return;Ticket {}});}",
    );
    for name in ["record", "variant"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
        assert_eq!(report.incomplete.len(), 1, "{name}: {report}");
        let item = &report.incomplete[0];
        assert_eq!(item.values.len(), 1, "{report}");
        assert_eq!(item.values[0].slot, 0);
        let nir::Term::Function { parameters, .. } = function.signature.callable_signature() else {
            panic!("signature")
        };
        assert_eq!(item.values[0].ty, parameters[0].ty);
        assert!(
            report.temporaries.is_empty(),
            "unfinished object is not a complete temporary: {report}"
        );
    }
}

#[test]
fn normalized_capture_preparation_keeps_completed_captures_on_exit() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::{Expr, ExprKind as K};
    let package = compile(
        "struct Ticket {} fn pair(a:Ticket,b:Ticket) {} fn run(a:Ticket,b:Ticket) {let action=|| pair(a,b);} fn stop() {return;}",
    );
    fn closure(expr: &mut Expr) -> &mut Expr {
        if matches!(expr.kind, K::Closure { .. }) {
            return expr;
        }
        match &mut expr.kind {
            K::Block { statements, .. } => closure(&mut statements[0]),
            K::Let { value, .. } | K::Discard(value) => closure(value),
            _ => panic!("closure expected"),
        }
    }
    // Synthesized capture expressions are represented in NIR; source capture
    // inference currently supplies lexical places. Exercise the normalized path
    // without inventing surface capture syntax.
    let mut function = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .clone();
    let stop = package.functions.iter().find(|f| f.name == "stop").unwrap();
    let K::Closure { captures, .. } = &mut closure(&mut function.body).kind else {
        unreachable!()
    };
    assert_eq!(captures.len(), 2);
    captures[1].value = Some(Box::new(stop.body.clone()));
    let before = function.to_string();
    assert!(analyze_in_package(&package, &function, |_| Ok(Delivery::Move), 1).is_err());
    let report = analyze_in_package(&package, &function, |_| Ok(Delivery::Move), 10000).unwrap();
    assert_eq!(report.incomplete.len(), 1, "{report}");
    assert_eq!(report.incomplete[0].values.len(), 1);
    assert_eq!(report.incomplete[0].values[0].slot, 0);
    assert!(
        report.closures.is_empty(),
        "environment never completed: {report}"
    );
    assert_eq!(
        report.cleanups.len(),
        1,
        "only uncaptured b remains: {report}"
    );
    assert_eq!(
        report.cleanups[0].place.root.local,
        function.parameters[1].index
    );
    assert_eq!(function.to_string(), before);
}

#[test]
fn ordered_cleanup_interleaves_pending_values_with_lexical_locals() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Outer {} struct Inner {} struct First {} struct Second {} fn run(a:First,b:Second) {let outer=Outer {}; let result=(a,do {let inner=Inner {}; (b,do {return; Second {}})});}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
    let site = report
        .cleanup_sites
        .iter()
        .find(|s| s.reason == CleanupReason::Return)
        .unwrap();
    let names: Vec<_> = site
        .actions
        .iter()
        .map(|action| {
            let ty = match action {
                CleanupAction::Owned { ty, .. } => ty,
                CleanupAction::Temporary { temporary, .. } => &temporary.ty,
                CleanupAction::Prepared { value, .. } => &value.ty,
                CleanupAction::Planned { .. } => panic!("move-only analysis must not plan Drop"),
                CleanupAction::Alternatives(_) => {
                    panic!("non-enum fixture has no residual alternatives")
                }
            };
            let nir::Term::Nominal { declaration, .. } = ty else {
                panic!("nominal cleanup")
            };
            package
                .structs
                .iter()
                .find(|s| s.declaration == *declaration)
                .unwrap()
                .name
                .as_str()
        })
        .collect();
    assert_eq!(names, ["Second", "Inner", "First", "Outer"], "{report}");
    assert!(matches!(site.actions[0], CleanupAction::Prepared { .. }));
    assert!(matches!(site.actions[1], CleanupAction::Owned { .. }));
    assert!(matches!(site.actions[2], CleanupAction::Prepared { .. }));
    assert!(site.to_string().contains("(reason return)"));
}

#[test]
fn cleanup_candidates_retain_whole_objects_after_last_field_move() {
    use elaboration::verification::moves::{CleanupAction, Delivery, analyze_in_package};
    use elaboration::verification::ownership::Availability;
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} fn take(value:Ticket) {} fn pass(value:Pair) {} fn fields(value:Pair) {take(value.left);take(value.right);} fn whole(value:Pair) {pass(value);} fn conditional(value:Pair,flag:bool) {if flag {pass(value);}}",
    );
    for name in ["fields", "whole", "conditional"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            10000,
        )
        .unwrap();
        let id = function.parameters[0];
        let candidates: Vec<_> = report
            .cleanup_sites
            .iter()
            .filter(|s| {
                s.reason != elaboration::verification::moves::CleanupReason::PredicateFailure
            })
            .flat_map(|s| &s.actions)
            .filter_map(|action| match action {
                CleanupAction::Owned { remaining, .. }
                    if remaining.place.root.local == id.index
                        && remaining.place.root.owner == u64::from(id.owner) =>
                {
                    Some(remaining)
                }
                _ => None,
            })
            .collect();
        if name == "whole" {
            assert!(candidates.is_empty(), "{report}");
            continue;
        }
        assert_eq!(candidates.len(), 1, "{report}");
        let tree = candidates[0];
        if name == "fields" {
            assert_eq!(tree.availability, Availability::Present);
            assert!(
                tree.children.as_ref().unwrap().is_empty(),
                "parent token remains: {report}"
            );
            assert!(
                report.cleanups.iter().all(|cleanup| report
                    .cleanup_sites
                    .iter()
                    .any(|site| site.reason
                        == elaboration::verification::moves::CleanupReason::PredicateFailure
                        && site.source == cleanup.source)),
                "leaf-only view omits the parent"
            );
        } else {
            assert_eq!(tree.availability, Availability::Conditional);
            assert!(
                tree.children
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|child| child.availability == Availability::Conditional)
            );
        }
    }
}

#[test]
fn replacement_keeps_old_partial_hierarchy_and_self_move_does_not_drop_it() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} fn take(value:Ticket) {} fn replace() {var value=Pair {left:Ticket {},right:Ticket {}};take(value.left);value=Pair {left:Ticket {},right:Ticket {}};} fn self_move() {var value=Pair {left:Ticket {},right:Ticket {}};value=value;}",
    );
    for name in ["replace", "self_move"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
        let replacements: Vec<_> = report
            .cleanup_sites
            .iter()
            .filter(|s| s.reason == CleanupReason::Replacement)
            .collect();
        if name == "self_move" {
            assert!(replacements.is_empty(), "{report}");
            continue;
        }
        assert_eq!(replacements.len(), 1);
        let CleanupAction::Owned { remaining, .. } = &replacements[0].actions[0] else {
            panic!("old owner")
        };
        let children = remaining.children.as_ref().unwrap();
        assert_eq!(
            children.len(),
            1,
            "only right remains in old value: {report}"
        );
        assert_eq!(children[0].place.fields, [1]);
        let final_site = report.cleanup_sites.last().unwrap();
        let CleanupAction::Owned { remaining, .. } = &final_site.actions[0] else {
            panic!("new owner")
        };
        assert_eq!(
            remaining
                .children
                .as_ref()
                .unwrap()
                .iter()
                .map(|c| c.place.fields.clone())
                .collect::<Vec<_>>(),
            [vec![1], vec![0]]
        );
    }
}

#[test]
fn break_cleanup_preserves_outer_preparations_and_orders_inner_ones() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Ticket {} fn pair(a:Ticket,b:Ticket) {} fn run(value:Ticket) {pair(value,do {while true {let local=Ticket {};pair(Ticket {},do {let newer=Ticket {};break;Ticket {}});}Ticket {}});}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
    let site = report
        .cleanup_sites
        .iter()
        .find(|s| s.reason == CleanupReason::Break)
        .unwrap();
    assert_eq!(site.actions.len(), 3, "{report}");
    assert!(matches!(site.actions[0], CleanupAction::Owned { .. }));
    assert!(matches!(site.actions[1], CleanupAction::Prepared { .. }));
    assert!(matches!(site.actions[2], CleanupAction::Owned { .. }));
    assert_eq!(
        report.cleanup_sites.len(),
        1,
        "loop locals must not be scheduled twice: {report}"
    );
}

#[test]
fn temporary_cleanup_candidates_end_after_assignment_and_before_local_scope_end() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, TemporaryBoundary, analyze_in_package,
    };
    let package = compile(
        "struct Holder {flag:bool} fn make()->Holder {Holder {flag:true}} fn run() {var picked=false; picked=make().flag;} fn tail()->bool {let local=Holder {flag:false};make().flag}",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    for name in ["run", "tail"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, policy, 10000).unwrap();
        let temp = report
            .cleanup_sites
            .iter()
            .position(|s| {
                s.actions
                    .iter()
                    .any(|a| matches!(a, CleanupAction::Temporary { .. }))
            })
            .unwrap();
        let retirement = report
            .cleanup_sites
            .iter()
            .position(|s| s.reason == CleanupReason::ScopeEnd)
            .unwrap();
        assert!(temp < retirement, "{report}");
        if name == "run" {
            let replacement = report
                .cleanup_sites
                .iter()
                .position(|s| s.reason == CleanupReason::Replacement)
                .unwrap();
            assert!(
                replacement < temp,
                "RHS owner must survive the assignment action: {report}"
            );
        }
        let CleanupAction::Temporary { temporary, .. } = &report.cleanup_sites[temp].actions[0]
        else {
            panic!("temporary")
        };
        assert_eq!(
            temporary.scope.boundary,
            if name == "tail" {
                TemporaryBoundary::FunctionResult
            } else {
                TemporaryBoundary::FullExpression
            }
        );
    }
}

#[test]
fn abrupt_exit_interleaves_projection_owner_prepared_value_and_locals() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Ticket {} struct Holder {item:Ticket,flag:bool} fn make()->Holder {Holder {item:Ticket {},flag:true}} fn pair(a:Ticket,b:Ticket) {} fn run() {let outer=Ticket {};pair(make().item,do {let inner=Ticket {};return;Ticket {}});}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 10000).unwrap();
    let site = report
        .cleanup_sites
        .iter()
        .find(|s| s.reason == CleanupReason::Return)
        .unwrap();
    assert_eq!(site.actions.len(), 4, "{report}");
    assert!(matches!(site.actions[0], CleanupAction::Owned { .. }));
    assert!(matches!(site.actions[1], CleanupAction::Prepared { .. }));
    let CleanupAction::Temporary { remaining, .. } = &site.actions[2] else {
        panic!("parent temporary: {report}")
    };
    assert_eq!(
        remaining
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.place.fields.clone())
            .collect::<Vec<_>>(),
        [vec![1]]
    );
    assert!(matches!(site.actions[3], CleanupAction::Owned { .. }));
    assert_eq!(
        report
            .cleanup_sites
            .iter()
            .filter(|s| s.reason != CleanupReason::PredicateFailure)
            .count(),
        1,
        "aborted full expression must not schedule cleanup again: {report}"
    );
}

#[test]
fn temporary_presence_follows_live_branches_and_return_does_not_consume_siblings() {
    use elaboration::verification::moves::{CleanupAction, Delivery, analyze_in_package};
    use elaboration::verification::ownership::Availability;
    let package = compile(
        "struct Holder {flag:bool} fn make()->Holder {Holder {flag:true}} fn branch(flag:bool) {let picked=if flag {make().flag} else {false};} fn sibling(flag:bool) {let pair=(make().flag,do {if flag {return;} false});}",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    for name in ["branch", "sibling"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, policy, 10000).unwrap();
        let temps: Vec<_> = report
            .cleanup_sites
            .iter()
            .flat_map(|s| &s.actions)
            .filter_map(|a| {
                if let CleanupAction::Temporary {
                    presence,
                    temporary,
                    ..
                } = a
                {
                    Some((*presence, temporary.source))
                } else {
                    None
                }
            })
            .collect();
        if name == "branch" {
            assert_eq!(temps.len(), 1, "{report}");
            assert_eq!(temps[0].0, Availability::Conditional);
        } else {
            assert_eq!(temps.len(), 2, "two disjoint exit paths: {report}");
            assert_eq!(temps[0], temps[1]);
            assert_eq!(temps[0].0, Availability::Present);
        }
    }
}

#[test]
fn conditions_end_their_temporaries_before_branch_and_match_keeps_subject_through_arm() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, TemporaryBoundary, analyze_in_package,
    };
    let package = compile(
        "struct Holder {flag:bool} fn make()->Holder {Holder {flag:true}} fn condition() {if make().flag {let local=Holder {flag:false};}} fn matching() {make() match {_=>{let local=Holder {flag:false};}}} fn guarded() {make() match {_ if false=>(),_=>()}} ",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    for name in ["condition", "matching", "guarded"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, policy, 10000).unwrap();
        let temps: Vec<_> = report
            .cleanup_sites
            .iter()
            .enumerate()
            .flat_map(|(index, s)| {
                s.actions.iter().filter_map(move |a| {
                    if let CleanupAction::Temporary { temporary, .. } = a {
                        Some((index, temporary))
                    } else {
                        None
                    }
                })
            })
            .collect();
        assert_eq!(temps.len(), 1, "{name}: {report}");
        let (temp, description) = temps[0];
        if name == "condition" {
            assert_eq!(description.scope.boundary, TemporaryBoundary::Condition);
            let local = report
                .cleanup_sites
                .iter()
                .position(|s| s.reason == CleanupReason::ScopeEnd)
                .unwrap();
            assert!(temp < local, "{report}");
        } else {
            assert_eq!(description.scope.boundary, TemporaryBoundary::Match);
            if name == "matching" {
                let local = report
                    .cleanup_sites
                    .iter()
                    .position(|s| s.reason == CleanupReason::ScopeEnd)
                    .unwrap();
                assert!(local < temp, "{report}");
            }
        }
    }
}

#[test]
fn loop_edges_keep_outer_temporaries_and_retire_iteration_temporaries_once() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Holder {flag:bool} fn make()->Holder {Holder {flag:true}} fn pair(a:bool,b:bool) {} fn outer() {pair(make().flag,do {while true {break;} false});} fn inner() {while true {pair(make().flag,do {break;false});}} fn repeated(flag:bool) {while flag {pair(make().flag,do {continue;false});}} ",
    );
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    for name in ["outer", "inner", "repeated"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, policy, 20000).unwrap();
        let sites: Vec<_> = report
            .cleanup_sites
            .iter()
            .filter(|s| {
                s.actions
                    .iter()
                    .any(|a| matches!(a, CleanupAction::Temporary { .. }))
            })
            .collect();
        assert_eq!(sites.len(), 1, "{name}: {report}");
        assert_eq!(
            sites[0].reason,
            match name {
                "outer" => CleanupReason::TemporaryEnd,
                "inner" => CleanupReason::Break,
                _ => CleanupReason::Continue,
            }
        );
    }
}

#[test]
fn closure_temporary_cleanup_stays_in_its_invocation_and_retains_caller_owner() {
    use elaboration::verification::moves::{
        CleanupAction, Delivery, TemporaryBoundary, analyze_in_package,
    };
    let package = compile(
        "struct Holder {flag:bool} fn make()->Holder {Holder {flag:true}} fn pair(a:bool,b:bool) {} fn run() {let action=|| make().flag;pair(make().flag,action());}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(
        &package,
        function,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        20000,
    )
    .unwrap();
    let outer: Vec<_> = report
        .cleanup_sites
        .iter()
        .filter(|s| s.reason != elaboration::verification::moves::CleanupReason::PredicateFailure)
        .flat_map(|s| &s.actions)
        .filter_map(|a| {
            if let CleanupAction::Temporary {
                identity,
                temporary,
                ..
            } = a
            {
                Some((*identity, temporary))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(outer.len(), 1, "{report}");
    assert_eq!(outer[0].1.scope.boundary, TemporaryBoundary::FullExpression);
    assert_eq!(report.closures.len(), 2, "latent and actual invocation");
    for closure in &report.closures {
        let inner: Vec<_> = closure
            .invocation
            .cleanup_sites
            .iter()
            .filter(|s| {
                s.reason != elaboration::verification::moves::CleanupReason::PredicateFailure
            })
            .flat_map(|s| &s.actions)
            .filter_map(|a| {
                if let CleanupAction::Temporary {
                    identity,
                    temporary,
                    ..
                } = a
                {
                    Some((*identity, temporary))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(inner.len(), 1, "{report}");
        assert_eq!(inner[0].1.scope.boundary, TemporaryBoundary::FunctionResult);
        assert_ne!(inner[0].0, outer[0].0);
    }
}

#[test]
fn branch_specific_temporary_candidates_keep_distinct_identities() {
    use elaboration::verification::moves::{CleanupAction, Delivery, analyze_in_package};
    use elaboration::verification::ownership::Availability;
    let package = compile(
        "struct Holder {flag:bool} fn make()->Holder {Holder {flag:true}} fn run(flag:bool) {let picked=if flag {make().flag} else {make().flag};}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let policy = |ty: &nir::Term| {
        Ok(if matches!(ty, nir::Term::Primitive(_)) {
            Delivery::Copy
        } else {
            Delivery::Move
        })
    };
    let before = function.to_string();
    assert!(analyze_in_package(&package, function, policy, 1).is_err());
    let report = analyze_in_package(&package, function, policy, 10000).unwrap();
    let candidates: Vec<_> = report
        .cleanup_sites
        .iter()
        .flat_map(|s| &s.actions)
        .filter_map(|a| {
            if let CleanupAction::Temporary {
                identity, presence, ..
            } = a
            {
                Some((*identity, *presence))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(candidates.len(), 2, "{report}");
    assert_ne!(candidates[0].0, candidates[1].0);
    assert!(
        candidates
            .iter()
            .all(|(_, p)| *p == Availability::Conditional)
    );
    assert_eq!(
        report.to_string(),
        analyze_in_package(&package, function, policy, 10000)
            .unwrap()
            .to_string()
    );
    assert_eq!(function.to_string(), before);
}

#[test]
fn record_patterns_keep_one_residual_parent_after_partial_or_complete_field_transfer() {
    use elaboration::verification::moves::{CleanupAction, Delivery, analyze_in_package};
    use elaboration::verification::ownership::Availability;
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} fn take(value:Ticket) {} fn partial(value:Pair) {let {left}=value;take(left);} fn all(value:Pair) {let {left,right}=value;take(left);take(right);} fn whole(value:Pair) {value match {owned=>{let _=owned;}}}",
    );
    for name in ["partial", "all", "whole"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000).unwrap();
        let candidates: Vec<_> = report
            .cleanup_sites
            .iter()
            .flat_map(|s| &s.actions)
            .filter_map(|a| {
                if let CleanupAction::Temporary {
                    temporary,
                    remaining,
                    ..
                } = a
                {
                    Some((temporary, remaining))
                } else {
                    None
                }
            })
            .collect();
        if name == "whole" {
            assert!(
                candidates.is_empty(),
                "whole binding owns the subject: {report}"
            );
            continue;
        }
        assert_eq!(
            candidates.len(),
            1,
            "no independent duplicate field cleanup: {report}"
        );
        let (temporary, remaining) = candidates[0];
        let nir::Term::Nominal { declaration, .. } = &temporary.ty else {
            panic!("nominal")
        };
        assert_eq!(
            package
                .structs
                .iter()
                .find(|s| s.declaration == *declaration)
                .unwrap()
                .name,
            "Pair"
        );
        assert_eq!(remaining.availability, Availability::Present);
        let children = remaining.children.as_ref().unwrap();
        if name == "all" {
            assert!(
                children.is_empty(),
                "parent survives the last field: {report}"
            );
        } else {
            assert_eq!(children.len(), 1);
            assert_eq!(children[0].place.fields, [1]);
        }
    }
}

#[test]
fn nested_generic_patterns_retain_each_enclosing_object_and_exact_residual_path() {
    use elaboration::verification::moves::{CleanupAction, Delivery, analyze_in_package};
    let package = tuple_support::compile_with_tuple(
        "struct Ticket {} struct Wrap where T {value:T,tag:Ticket} fn take(value:Ticket) {} fn run(value:Wrap<Tuple<Ticket,Ticket>>) {let {value:(picked,_),tag}=value;take(picked);take(tag);}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000).unwrap();
    let candidates: Vec<_> = report
        .cleanup_sites
        .iter()
        .flat_map(|s| &s.actions)
        .filter_map(|a| {
            if let CleanupAction::Temporary { remaining, .. } = a {
                Some(remaining)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(candidates.len(), 1, "{report}");
    let parent = candidates[0];
    let fields = parent.children.as_ref().unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].place.fields, [0]);
    let tuple = fields[0].children.as_ref().unwrap();
    assert_eq!(tuple.len(), 1);
    assert_eq!(tuple[0].place.fields, [0, 1]);
    assert!(
        tuple[0].children.as_ref().unwrap().is_empty(),
        "Ticket is a completed empty aggregate"
    );
}

#[test]
fn enum_pattern_cleanup_keeps_selected_variant_with_its_residual_payload() {
    use elaboration::verification::moves::{CleanupAction, Delivery, analyze_in_package};
    use elaboration::verification::ownership::Availability;
    let package = compile(
        "struct Ticket {} enum Choice {empty,pair(Ticket,Ticket)} fn take(value:Ticket) {} fn run(value:Choice) {value match {.empty=>(),.pair(item,_)=>take(item)}}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000).unwrap();
    let mut cases = std::collections::BTreeSet::new();
    let mut count = 0;
    for action in report
        .cleanup_sites
        .iter()
        .filter(|s| s.reason != elaboration::verification::moves::CleanupReason::PredicateFailure)
        .flat_map(|s| &s.actions)
    {
        if let CleanupAction::Temporary {
            presence,
            variants,
            remaining,
            ..
        } = action
        {
            count += 1;
            assert_eq!(*presence, Availability::Conditional);
            assert_eq!(variants.len(), 1, "{report}");
            assert!(variants[0].fields.is_empty());
            cases.insert(variants[0].index);
            let fields = remaining.children.as_ref().unwrap();
            if variants[0].index == 0 {
                assert!(fields.is_empty());
            } else {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].place.fields, [1]);
            }
        }
    }
    assert_eq!(
        count, 2,
        "one conditional owner per selected variant: {report}"
    );
    assert_eq!(cases, std::collections::BTreeSet::from([0, 1]));
    assert!(
        report
            .to_string()
            .contains("(variant-selection (fields) (index 1))")
    );
}

#[test]
fn nested_enum_selection_is_relative_to_the_whole_subject() {
    use elaboration::verification::moves::{CleanupAction, Delivery, analyze_in_package};
    let package = tuple_support::compile_with_tuple(
        "struct Ticket {} enum Choice where T {none,some(T,T)} fn take(value:Ticket) {} fn run(value:Tuple<Choice<Ticket>,Ticket>) {value match {(.some(item,_),last)=>{take(item);take(last);},_=>()}} ",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000).unwrap();
    let selected: Vec<_> = report
        .cleanup_sites
        .iter()
        .filter(|s| s.reason != elaboration::verification::moves::CleanupReason::PredicateFailure)
        .flat_map(|s| &s.actions)
        .filter_map(|a| {
            if let CleanupAction::Temporary {
                variants,
                remaining,
                ..
            } = a
            {
                (!variants.is_empty()).then_some((variants, remaining))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(selected.len(), 1, "{report}");
    let (variants, tree) = selected[0];
    assert_eq!(variants[0].fields, [0]);
    assert_eq!(variants[0].index, 1);
    let selected_enum = &tree.children.as_ref().unwrap()[0];
    assert_eq!(selected_enum.place.fields, [0]);
    assert_eq!(
        selected_enum.children.as_ref().unwrap()[0].place.fields,
        [0, 1]
    );
}

#[test]
fn returning_from_pattern_arm_retires_bindings_before_the_subject_owner() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Ticket {} struct Pair {left:Ticket,right:Ticket} fn run(value:Pair) {let outer=Ticket {};value match {{left}=>{let inner=Ticket {};return;}}}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000).unwrap();
    let site = report
        .cleanup_sites
        .iter()
        .find(|s| s.reason == CleanupReason::Return)
        .unwrap();
    assert_eq!(
        site.actions.len(),
        4,
        "inner, binding, residual parent, outer: {report}"
    );
    assert!(matches!(site.actions[0], CleanupAction::Owned { .. }));
    assert!(matches!(site.actions[1], CleanupAction::Owned { .. }));
    let CleanupAction::Temporary { remaining, .. } = &site.actions[2] else {
        panic!("parent")
    };
    assert_eq!(remaining.children.as_ref().unwrap().len(), 1);
    assert_eq!(remaining.children.as_ref().unwrap()[0].place.fields, [1]);
    assert!(matches!(site.actions[3], CleanupAction::Owned { .. }));
}

#[test]
fn rejected_pattern_guards_and_let_else_keep_the_original_subject_on_failure() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Ticket {} enum Choice {none,pair(Ticket,Ticket)} fn take(value:Ticket) {} fn guarded(value:Choice) {value match {.pair(item,_) if false=>take(item),_=>()}} fn binding(value:Choice) {let .pair(item,_)=value else {return;};take(item);}",
    );
    for name in ["guarded", "binding"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let report = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000).unwrap();
        let candidates: Vec<_> = report
            .cleanup_sites
            .iter()
            .flat_map(|s| {
                s.actions.iter().filter_map(move |a| {
                    if let CleanupAction::Temporary {
                        variants,
                        remaining,
                        ..
                    } = a
                    {
                        Some((s.reason, variants, remaining))
                    } else {
                        None
                    }
                })
            })
            .collect();
        if name == "guarded" {
            assert_eq!(candidates.len(), 1, "{report}");
            assert!(
                candidates[0].1.is_empty(),
                "failed selected-variant evidence must not escape"
            );
            assert!(
                candidates[0].2.children.is_none(),
                "whole unknown enum remains owned"
            );
            assert!(report.decompositions.is_empty());
        } else {
            assert_eq!(
                candidates.len(),
                2,
                "success/failure responsibilities: {report}"
            );
            let failed = candidates
                .iter()
                .find(|(reason, _, _)| *reason == CleanupReason::Return)
                .unwrap();
            assert!(failed.1.is_empty());
            assert!(failed.2.children.is_none());
            let success = candidates
                .iter()
                .find(|(reason, _, _)| *reason == CleanupReason::TemporaryEnd)
                .unwrap();
            assert_eq!(success.1[0].index, 1);
            assert_eq!(success.2.children.as_ref().unwrap()[0].place.fields, [1]);
        }
    }
}

#[test]
fn invalid_normalized_variant_layout_fails_without_changing_the_valid_subject_plan() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::{Expr, ExprKind as K, Pattern};
    let package = compile(
        "struct Ticket {} enum Choice {none,some(Ticket)} fn take(value:Ticket) {} fn run(value:Choice) {value match {.none=>(),.some(item)=>take(item)}}",
    );
    let function = package.functions.iter().find(|f| f.name == "run").unwrap();
    let baseline = analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000)
        .unwrap()
        .to_string();
    let mut invalid = function.clone();
    fn matching(expr: &mut Expr) -> &mut Expr {
        if matches!(expr.kind, K::Match { .. }) {
            return expr;
        }
        match &mut expr.kind {
            K::Block { statements, result } => {
                if let Some(result) = result {
                    matching(result)
                } else {
                    matching(&mut statements[0])
                }
            }
            K::Discard(value) => matching(value),
            _ => panic!("match expected"),
        }
    }
    let K::Match { arms, .. } = &mut matching(&mut invalid.body).kind else {
        unreachable!()
    };
    let Pattern::Variant { index, .. } = &mut arms[1].pattern else {
        panic!("variant expected")
    };
    *index = usize::MAX;
    let before = invalid.to_string();
    let error = analyze_in_package(&package, &invalid, |_| Ok(Delivery::Move), 20000).unwrap_err();
    assert!(error.reason.contains("out of bounds"), "{error}");
    assert_eq!(invalid.to_string(), before);
    assert!(analyze_in_package(&package, function, |_| Ok(Delivery::Move), 1).is_err());
    assert_eq!(
        baseline,
        analyze_in_package(&package, function, |_| Ok(Delivery::Move), 20000)
            .unwrap()
            .to_string()
    );
}

#[test]
fn local_pointer_access_uses_shared_owned_place_state() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run() { let value = false; let p = value.ref; let q = p; p.* = true; let seen = q.*; }",
            true,
        ),
        (
            "struct Cell { value:bool, } fn run() { let cell = Cell { value:false }; let p = cell.ref; let field = p.value; field.* = true; let seen = field.*; }",
            true,
        ),
        (
            "fn run(flag:bool) { let first = false; let second = true; var p = first.ref; if flag { p = second.ref; } let seen = p.*; }",
            true,
        ),
        (
            "fn run(flag:bool) { let value = false; var p = value.ref; if flag { p = value.ref; } let seen = p.*; }",
            true,
        ),
        (
            "fn run() { let p = do { let value = true; value.ref }; let seen = p.*; }",
            false,
        ),
        ("fn run(p:*bool) { let seen = p.*; }", false),
        (
            "fn call(p:*bool) {} fn run() { let value = false; let p = value.ref; call(p); }",
            true,
        ),
    ];
    for (source, accepted) in cases {
        let package = compile(source);
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
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
        );
        assert_eq!(result.is_ok(), accepted, "{source}: {result:?}");
        if let Err(error) = result {
            assert!(
                error.reason.contains("pointer") || error.reason.contains("memory contract"),
                "{error}"
            );
        }
    }
}

#[test]
fn local_pointer_access_rejects_moved_parent_and_recovers_after_replacement() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (replacement, accepted) in [("", false), ("cell = Cell { value:true };", true)] {
        let package = compile(&format!(
            "struct Cell {{ value:bool, }} fn run() {{ var cell = Cell {{ value:false }}; let p = cell.value.ref; let moved = cell; {replacement} let seen = p.*; }}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let before = package.to_string();
        let result = analyze_in_package(
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
        );
        assert_eq!(result.is_ok(), accepted, "{result:?}");
        assert_eq!(package.to_string(), before);
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn local_pointer_scope_and_indirect_slot_updates_discard_stale_provenance() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run() { let value = false; var p = value.ref; while true { let inner = true; p = inner.ref; break; } let seen = p.*; }",
            false,
        ),
        (
            "fn run(flag:bool) { let value = false; let p = value.ref; while true { p.* = true; if flag { break; } } let seen = p.*; }",
            true,
        ),
        (
            "fn run() { let first = false; let second = true; var p = first.ref; let slot = p.ref; slot.* = second.ref; let seen = p.*; }",
            true,
        ),
        (
            "fn run(flag:bool) { let first = false; let second = true; var p = first.ref; if flag { p = second.ref; } p = first.ref; let seen = p.*; }",
            true,
        ),
    ];
    for (source, accepted) in cases {
        let package = compile(source);
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let delivery = |ty: &nir::Term| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        };
        assert!(analyze_in_package(&package, function, delivery, 0).is_err());
        let result = analyze_in_package(&package, function, delivery, 100_000);
        assert_eq!(result.is_ok(), accepted, "{source}: {result:?}");
    }
}

#[test]
fn local_pointer_call_boundary_does_not_hide_dependencies_in_aggregates() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (source, accepted) in [
        (
            "struct Holder { pointer:*bool, } fn consume(holder:Holder) {} fn run() { let value = false; consume(Holder { pointer:value.ref }); }",
            true,
        ),
        (
            "struct Holder { pointer:*bool, } fn consume(holder:Holder) {} fn run() { let value = false; let holder = Holder { pointer:value.ref }; consume(holder); }",
            true,
        ),
        (
            "fn action() { let value = false; let pointer = value.ref; } fn run() { let value = false; let unused = do { let pointer = value.ref; false }; action(); }",
            true,
        ),
        (
            "struct Holder { pointer:*bool, } fn consume(holder:Holder) { let seen=holder.pointer.*; } fn run() { let value=false; consume(Holder {pointer:value.ref}); }",
            true,
        ),
        (
            "struct Holder { pointer:*bool, } fn consume(holder:Holder) { holder.pointer.* = true; } fn run() { let value=false; let holder=Holder {pointer:value.ref}; consume(holder); }",
            true,
        ),
        (
            "struct Holder { pointer:*bool, } fn borrow(holder:Holder)->*bool {holder.pointer} fn run() {let value=false;let seen=borrow(Holder {pointer:value.ref}).*;}",
            true,
        ),
        (
            "struct Holder { pointer:*bool, } fn consume(holder:Holder) {let seen=holder.pointer.*;} fn run() {let holder=do {let value=false;Holder {pointer:value.ref}};consume(holder);}",
            false,
        ),
    ] {
        let package = compile(source);
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
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
        );
        if accepted {
            assert!(result.is_ok(), "{source}: {result:?}");
            continue;
        }
        let error = result.unwrap_err();
        assert!(
            error.reason.contains("field provenance")
                || error.reason.contains("unavailable")
                || error.reason.contains("memory contract"),
            "{source}: {error}"
        );
    }
}

#[test]
fn memory_summaries_check_call_targets_and_permit_sequential_aliases() {
    use elaboration::verification::{
        memory,
        moves::{Delivery, analyze_in_package},
    };
    let cases = [
        (
            "fn access(first:*bool, second:*bool) -> bool { first.* = true; second.* } fn run() { let value=false; let seen=access(value.ref,value.ref); }",
            true,
        ),
        (
            "struct Cell { value:bool, fn set(*self) { self.value.* = true; } } fn run() { let cell=Cell {value:false}; cell.set(); let seen=cell.value; }",
            true,
        ),
        (
            "struct Cell { value:bool, } fn access(p:*bool) -> bool { p.* } fn run() { let cell=Cell {value:false}; let p=cell.value.ref; let moved=cell; let seen=access(p); }",
            false,
        ),
        (
            "struct Cell { value:bool, } fn access(p:*bool, flag:bool) -> bool { p.* } fn run() { let cell=Cell {value:false}; let p=cell.value.ref; let seen=access(p,do { let moved=cell; true }); }",
            false,
        ),
        (
            "fn access(p:*bool, flag:bool) -> bool { if flag { p.* = true; } p.* } fn run() { let value=false; let seen=access(value.ref,true); }",
            true,
        ),
        (
            "struct Cell { value:bool, } fn access(p:*bool, flag:bool) -> bool { p.* } fn run() { let first=Cell {value:false}; let second=Cell {value:true}; var p=first.value.ref; let seen=access(p,do { p=second.value.ref; let moved=first; true }); }",
            false,
        ),
    ];
    for (source, accepted) in cases {
        let package = compile(source);
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let before = package.to_string();
        let result = analyze_in_package(
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
        );
        assert_eq!(result.is_ok(), accepted, "{source}: {result:?}");
        assert_eq!(package.to_string(), before);
        let callee = package
            .functions
            .iter()
            .find(|f| f.name == "access" || f.name == "set")
            .unwrap();
        let summary = memory::infer(callee, &mut 10000).unwrap();
        assert!(!summary.accesses.is_empty());
        assert!(summary.to_string().starts_with("(memory-summary "));
        assert!(memory::infer(callee, &mut 0).is_err());
        assert_eq!(summary, memory::infer(callee, &mut 10000).unwrap());
    }
}

#[test]
fn memory_summaries_do_not_turn_unknown_calls_or_pointer_escape_into_purity() {
    use elaboration::verification::memory;
    for source in [
        "fn other() {} fn inspect() { other(); }",
        "fn inspect() -> *bool { let local=true; local.ref }",
        "fn inspect(pointer:*(*bool), replacement:*bool) { pointer.* = replacement; }",
    ] {
        let package = compile(source);
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        assert!(memory::infer(function, &mut 10000).is_err(), "{source}");
    }
}

#[test]
fn composed_memory_summaries_rebase_fields_and_generic_instances() {
    use elaboration::verification::{
        memory,
        moves::{Delivery, analyze_in_package},
    };
    let package = compile(
        "struct Cell where T { value:bool, marker:T, } struct Outer { cell:Cell<bool>, } fn read(p:*bool) -> bool { p.* } fn relay(p:*Cell<T>) -> bool where T { read(p.value) } fn wrapper(p:*Outer) -> bool { relay(p.cell) } fn run() { let outer=Outer {cell:Cell<bool> {value:true,marker:false}}; let result=wrapper(outer.ref); }",
    );
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    let summary = memory::infer_in_package(&package, wrapper.declaration, &[], &mut 10000).unwrap();
    assert!(summary.accesses.iter().any(|access| access.read_value
        && access.target.parameter == 0
        && access.target.fields.len() == 2));
    assert_eq!(
        summary,
        memory::infer_in_package(&package, wrapper.declaration, &[], &mut 10000).unwrap()
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    analyze_in_package(
        &package,
        run,
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
    let relay = package
        .functions
        .iter()
        .find(|f| f.name == "relay")
        .unwrap();
    let bool_summary = memory::infer_in_package(
        &package,
        relay.declaration,
        &[nir::Term::Primitive(nir::Primitive::Bool)],
        &mut 10000,
    )
    .unwrap();
    let char_summary = memory::infer_in_package(
        &package,
        relay.declaration,
        &[nir::Term::Primitive(nir::Primitive::Char)],
        &mut 10000,
    )
    .unwrap();
    assert_ne!(bool_summary, char_summary);
    assert_ne!(bool_summary.to_string(), char_summary.to_string());
    assert!(memory::infer_in_package(&package, relay.declaration, &[], &mut 10000).is_err());
}

#[test]
fn composed_memory_summaries_close_empty_cycles_and_reject_missing_bodies() {
    use elaboration::verification::memory;
    let mut package = compile(
        "fn leaf(p:*bool) -> bool { p.* } fn wrapper(p:*bool) -> bool { leaf(p) } fn recursive(p:*bool) -> bool { recursive(p) } fn first(p:*bool) -> bool { second(p) } fn second(p:*bool) -> bool { first(p) }",
    );
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap()
        .declaration;
    for name in ["recursive", "first"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        assert!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 10000)
                .unwrap()
                .accesses
                .is_empty()
        );
    }
    assert!(memory::infer_in_package(&package, wrapper, &[], &mut 1).is_err());
    assert!(memory::infer_in_package(&package, wrapper, &[], &mut 10000).is_ok());
    package.functions.retain(|f| f.name != "leaf");
    assert!(
        memory::infer_in_package(&package, wrapper, &[], &mut 10000)
            .unwrap_err()
            .contains("missing")
    );
}

#[test]
fn composed_memory_summaries_verify_callee_signature_and_qualification_paths() {
    use elaboration::verification::memory;
    let mut package = compile(
        "fn leaf(p:*bool) -> bool { p.* = true; p.* } fn reorder(first:*bool, second:*bool) -> bool { leaf(second) } fn wrapper(p:*bool) -> bool { reorder(p,p) }",
    );
    let reorder = package
        .functions
        .iter()
        .find(|f| f.name == "reorder")
        .unwrap()
        .declaration;
    let summary = memory::infer_in_package(&package, reorder, &[], &mut 10000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .all(|access| access.target.parameter == 1)
    );
    assert!(summary.accesses.iter().any(|access| access.write));
    let wrapper = package
        .functions
        .iter_mut()
        .find(|f| f.name == "wrapper")
        .unwrap();
    let id = wrapper.declaration;
    let nir::program::ExprKind::Block {
        result: Some(result),
        ..
    } = &mut wrapper.body.kind
    else {
        panic!("block")
    };
    let nir::program::ExprKind::Call { callee, .. } = &mut result.kind else {
        panic!("call")
    };
    let nir::Term::Function { result, .. } = &mut callee.ty else {
        panic!("signature")
    };
    **result = nir::Term::Primitive(nir::Primitive::Char);
    assert!(
        memory::infer_in_package(&package, id, &[], &mut 10000)
            .unwrap_err()
            .contains("signature disagrees")
    );
}

#[test]
fn recursive_memory_fixed_point_propagates_permuted_reads_and_writes() {
    use elaboration::verification::{
        memory,
        moves::{Delivery, analyze_in_package},
    };
    let package = compile(
        "fn first(p:*bool,q:*bool,flag:bool) -> bool { if flag { second(q,p,false) } else { p.* } } fn second(a:*bool,b:*bool,flag:bool) -> bool { a.* = true; first(a,b,flag) } fn run() { let value=false; let result=first(value.ref,value.ref,true); }",
    );
    let first = package
        .functions
        .iter()
        .find(|f| f.name == "first")
        .unwrap();
    let before = package.to_string();
    let summary = memory::infer_in_package(&package, first.declaration, &[], &mut 100000).unwrap();
    for parameter in [0, 1] {
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.target.parameter == parameter && a.write)
        );
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.target.parameter == parameter && !a.write && a.read_value)
        );
    }
    assert_eq!(summary.accesses.len(), 4);
    assert_eq!(
        summary,
        memory::infer_in_package(&package, first.declaration, &[], &mut 100000).unwrap()
    );
    assert!(memory::infer_in_package(&package, first.declaration, &[], &mut 12).is_err());
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        100000,
    )
    .unwrap();
    assert_eq!(package.to_string(), before);
}

#[test]
fn recursive_memory_fixed_point_keeps_instances_separate_and_never_returns_partial_growth() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Cell where T { flag:bool, value:T, } fn read(p:*Cell<T>,again:bool)->bool where T { if again { read(p,false) } else { p.flag.* } } fn grow(p:*T)->bool where T { grow(p.ref) }",
    );
    let read = package.functions.iter().find(|f| f.name == "read").unwrap();
    let bools = memory::infer_in_package(
        &package,
        read.declaration,
        &[nir::Term::Primitive(nir::Primitive::Bool)],
        &mut 100000,
    )
    .unwrap();
    let chars = memory::infer_in_package(
        &package,
        read.declaration,
        &[nir::Term::Primitive(nir::Primitive::Char)],
        &mut 100000,
    )
    .unwrap();
    assert_ne!(bools, chars);
    assert!(
        bools
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.fields.len() == 1)
    );
    let grow = package.functions.iter().find(|f| f.name == "grow").unwrap();
    assert!(
        memory::infer_in_package(
            &package,
            grow.declaration,
            &[nir::Term::Primitive(nir::Primitive::Bool)],
            &mut 1000
        )
        .is_err()
    );
    assert_eq!(
        bools,
        memory::infer_in_package(
            &package,
            read.declaration,
            &[nir::Term::Primitive(nir::Primitive::Bool)],
            &mut 100000
        )
        .unwrap()
    );
}

#[test]
fn recursive_memory_fixed_point_uses_a_worklist_for_long_call_chains() {
    use elaboration::verification::memory;
    let mut source = String::new();
    for index in 0..150 {
        source.push_str(&format!(
            "fn step{index}(p:*bool)->bool {{ step{}(p) }} ",
            index + 1
        ));
    }
    source.push_str("fn step150(p:*bool)->bool { p.* }");
    let package = compile(&source);
    let first = package
        .functions
        .iter()
        .find(|f| f.name == "step0")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, first.declaration, &[], &mut 3_000_000).unwrap();
    assert_eq!(summary.accesses.len(), 1);
    assert!(summary.accesses[0].read_value);
    assert_eq!(summary.accesses[0].target.parameter, 0);
    assert!(memory::infer_in_package(&package, first.declaration, &[], &mut 100).is_err());
}

#[test]
fn recursive_memory_fixed_point_bounds_exponentially_growing_type_instances() {
    use elaboration::verification::memory;
    let package = tuple_support::compile_with_tuple(
        "fn grow()->bool where T { grow<Tuple<T,T>>() } fn read(p:*bool)->bool { p.* }",
    );
    let grow = package.functions.iter().find(|f| f.name == "grow").unwrap();
    let read = package.functions.iter().find(|f| f.name == "read").unwrap();
    let before = package.to_string();
    let error = memory::infer_in_package(
        &package,
        grow.declaration,
        &[nir::Term::Primitive(nir::Primitive::Bool)],
        &mut 1000,
    )
    .unwrap_err();
    assert!(error.contains("budget"), "{error}");
    assert_eq!(
        memory::infer_in_package(&package, read.declaration, &[], &mut 1000)
            .unwrap()
            .accesses
            .len(),
        1
    );
    assert_eq!(package.to_string(), before);
}

#[test]
fn pointer_target_sets_cover_branch_selection_fields_and_memory_calls() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let cases = [
        (
            "fn run(flag:bool) { let first=false; let second=true; let p=if flag { first.ref } else { second.ref }; p.* = true; let seen=p.*; }",
            true,
        ),
        (
            "struct Cell { value:bool, } fn touch(p:*bool) { p.* = true; } fn run(flag:bool) { let first=Cell {value:false}; let second=Cell {value:true}; var p=first.ref; if flag { p=second.ref; } touch(p.value); }",
            true,
        ),
        (
            "struct Cell { value:bool, } fn read(p:*bool)->bool {p.*} fn run(flag:bool) { let first=Cell {value:false}; let second=Cell {value:true}; var p=first.value.ref; if flag {p=second.value.ref;} let moved=second; let seen=read(p); }",
            false,
        ),
        (
            "fn run(flag:bool) { let first=false; var p=first.ref; if flag { let other=true; p=other.ref; } let seen=p.*; }",
            false,
        ),
        (
            "fn run(flag:bool) { let first=false; let second=true; var p=first.ref; while flag {p=second.ref;} let seen=p.*; }",
            true,
        ),
    ];
    for (source, accepted) in cases {
        let package = compile(source);
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let before = package.to_string();
        let policy = |ty: &nir::Term| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        };
        assert!(analyze_in_package(&package, run, policy, 0).is_err());
        let result = analyze_in_package(&package, run, policy, 100000);
        assert_eq!(result.is_ok(), accepted, "{source}: {result:?}");
        assert_eq!(package.to_string(), before);
    }
}

#[test]
fn pointer_target_set_indirect_slot_write_preserves_every_candidate() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for selected in ["p", "q"] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} fn run(flag:bool) {{let first=Cell {{value:false}}; let second=Cell {{value:true}}; var p=first.value.ref; var q=first.value.ref; let slot=if flag {{p.ref}} else {{q.ref}}; slot.* = second.value.ref; {moved} let seen={selected}.*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                moved.is_empty(),
                "{selected}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn pointer_target_sets_validate_each_candidate_after_many_branch_joins() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for unavailable in 0..=8 {
        let mut source = String::from("struct Cell { value:bool, } fn run(");
        source.push_str(
            &(0..8)
                .map(|i| format!("flag{i}:bool"))
                .collect::<Vec<_>>()
                .join(","),
        );
        source.push_str(") {");
        for i in 0..8 {
            source.push_str(&format!("let cell{i}=Cell {{value:false}};"));
        }
        source.push_str("var pointer=cell0.value.ref;");
        for i in 1..8 {
            source.push_str(&format!("if flag{i} {{pointer=cell{i}.value.ref;}}"));
        }
        if unavailable < 8 {
            source.push_str(&format!("let moved=cell{unavailable};"));
        }
        source.push_str("let value=pointer.*;}");
        let package = compile(&source);
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(
            result.is_ok(),
            unavailable == 8,
            "candidate {unavailable}: {result:?}"
        );
    }
}

#[test]
fn memory_defaults_use_prepared_pointer_parameters_without_reevaluation() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for source in [
        "fn read(p:*bool,.q:*bool=p)->bool {q.*} fn run() {let value=true; let result=read(value.ref);}",
        "fn read(p:*bool,.q:*bool=p,.r:*bool=q)->bool {r.*} fn run(flag:bool) {let first=true; let second=false; let p=if flag {first.ref} else {second.ref}; let result=read(p);}",
        "fn read(p:*bool,.q:*bool=do {let saved=p; do {let alias=saved; alias}})->bool {q.*} fn run() {let value=true; let result=read(value.ref);}",
    ] {
        let package = compile(source);
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        )
        .unwrap();
    }
}

#[test]
fn memory_summary_pointer_blocks_keep_parameter_paths_and_check_side_effects() {
    use elaboration::verification::memory;
    for body in [
        "let alias=do {let saved=p; saved}; alias.*",
        "(do {let saved=p; saved}).*",
        "leaf(do {let saved=p; saved})",
        "leaf(p)",
    ] {
        let package = compile(&format!(
            "fn leaf(p:*bool,.q:*bool=do {{let saved=p; saved}})->bool {{q.*}} fn wrapper(p:*bool)->bool {{{body}}}"
        ));
        let wrapper = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
        assert_eq!(summary.accesses.len(), 1, "{summary}");
        assert_eq!(summary.accesses[0].target.parameter, 0);
        assert!(summary.accesses[0].read_value);
    }
    // A result path is not permission to ignore the block's preceding effects.
    let package = compile(
        "fn leaf(p:*bool)->bool {p.*} fn wrapper(p:*bool)->bool {leaf(do {let local=true; let escaped=local.ref; p})}",
    );
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
    assert_eq!(summary.accesses.len(), 1, "{summary}");
    assert_eq!(summary.accesses[0].target.parameter, 0);
    let package = compile(
        "fn leaf(p:*bool)->bool {p.*} fn wrapper(p:*bool,slot:**bool)->bool {leaf(do {let local=true;slot.* = local.ref;p})}",
    );
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    assert!(memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).is_err());

    let package = compile(
        "struct Cell {value:bool,} fn write(p:*Cell) {(do {let saved=p; saved}).value.* = true;}",
    );
    let write = package
        .functions
        .iter()
        .find(|f| f.name == "write")
        .unwrap();
    let summary = memory::infer_in_package(&package, write.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary.accesses.iter().any(|access| access.write
            && access.target.parameter == 0
            && access.target.fields.len() == 1),
        "{summary}"
    );
}

#[test]
fn memory_defaults_pointer_blocks_cannot_revive_moved_owners() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in [false, true] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn leaf(p:*bool,.q:*bool=do {{let alias=p; alias}})->bool {{q.*}} fn run() {{let cell=Cell {{value:true}}; let p=cell.value.ref; {} let result=leaf(p);}}",
            if moved { "let moved=cell;" } else { "" }
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), !moved, "{result:?}");
    }
}

#[test]
fn memory_conditional_defaults_check_every_candidate_and_respect_overrides() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for unavailable in 0..3 {
        for overridden in [false, true] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} fn read(flag:bool,a:*Cell,b:*Cell,.q:*Cell=if flag {{a}} else {{b}})->bool {{q.value.*}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let spare=Cell {{value:true}}; let a=first.ref; let b=second.ref; {} let result=read(flag,a,b{});}}",
                match unavailable {
                    0 => "let moved=first;",
                    1 => "let moved=second;",
                    _ => "",
                },
                if overridden { ",.q=spare.ref" } else { "" },
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                unavailable == 2 || overridden,
                "owner {unavailable}, override {overridden}: {result:?}"
            );
        }
    }
}

#[test]
fn memory_conditional_targets_compose_fields_and_deduplicate_aliases() {
    use elaboration::verification::memory;
    for second in ["a", "b"] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn read(flag:bool,a:*Cell,b:*Cell,.q:*Cell=if flag {{a}} else {{{second}}})->bool {{q.value.*}} fn wrapper(flag:bool,a:*Cell,b:*Cell)->bool {{read(flag,a,b)}}"
        ));
        let wrapper = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
        let reads: Vec<_> = summary.accesses.iter().filter(|a| a.read_value).collect();
        assert_eq!(reads.len(), if second == "a" { 1 } else { 2 }, "{summary}");
        assert!(reads.iter().any(|a| a.target.parameter == 1));
        assert!(reads.iter().all(|a| a.target.fields.len() == 1));
        assert_eq!(reads.iter().any(|a| a.target.parameter == 2), second == "b");
        assert!(memory::infer_in_package(&package, wrapper.declaration, &[], &mut 1).is_err());
    }
}

#[test]
fn memory_conditional_indirect_branch_is_not_discarded() {
    use elaboration::verification::memory;
    for access in [false, true] {
        let package = compile(&format!(
            "fn read(flag:bool,known:*bool,slot:*(*bool))->bool {{let selected=if flag {{known}} else {{slot.*}}; {}}}",
            if access { "selected.*" } else { "true" },
        ));
        let read = package.functions.iter().find(|f| f.name == "read").unwrap();
        let result = memory::infer_in_package(&package, read.declaration, &[], &mut 100000);
        let summary = result.unwrap();
        assert_eq!(
            summary.accesses.len(),
            if access { 3 } else { 1 },
            "{summary}"
        );
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.target.parameter == 2 && a.target.through.is_empty())
        );
        if access {
            assert!(summary.accesses.iter().any(|a| a.target.parameter == 1));
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 2 && a.target.through == vec![vec![]])
            );
        }
    }
}

#[test]
fn memory_argument_dump_distinguishes_known_alternatives_from_unknown() {
    use elaboration::verification::memory::{ArgumentSummary, ParameterPlace};
    let mut summary = ArgumentSummary {
        input_pointer_exits: Default::default(),
        input_code_exits: Default::default(),
        pointer_fields: Default::default(),
        code_fields: Default::default(),
        may_reject: false,
        may_run_cleanup: false,
        accesses: vec![],
        result_targets: Some(
            [2, 1, 2]
                .into_iter()
                .map(|parameter| ParameterPlace {
                    parameter,
                    input_fields: vec![],
                    through: vec![],
                    conditions: Default::default(),
                    fields: vec![],
                })
                .collect(),
        ),
    };
    assert_eq!(
        summary.to_string(),
        "(argument-memory (accesses) (may-reject false) (may-run-cleanup false) (result-targets (parameter-place (parameter 1) (fields)) (parameter-place (parameter 2) (fields))) (code-fields))"
    );
    summary.result_targets = None;
    assert_eq!(
        summary.to_string(),
        "(argument-memory (accesses) (may-reject false) (may-run-cleanup false) (result-targets unknown) (code-fields))"
    );
}

#[test]
fn memory_defaults_check_source_reads_after_all_explicit_arguments() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (move_owner, override_default) in [(false, false), (true, false), (true, true)] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn read(p:*bool,flag:bool,.value:bool=p.*)->bool {{value}} fn run() {{let cell=Cell {{value:true}}; let p=cell.value.ref; let result=read(p,do {{{} true}}{});}}",
            if move_owner { "let moved=cell;" } else { "" },
            if override_default { ",.value=true" } else { "" }
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(
            result.is_ok(),
            !move_owner || override_default,
            "{result:?}"
        );
    }
}

#[test]
fn memory_defaults_compose_pointer_defaults_through_wrappers() {
    use elaboration::verification::{
        memory,
        moves::{Delivery, analyze_in_package},
    };
    let package = compile(
        "fn leaf(p:*bool,.q:*bool=p)->bool {q.*} fn wrapper(p:*bool)->bool {leaf(p)} fn run() {let value=true; let result=wrapper(value.ref);}",
    );
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.read_value && access.target.parameter == 0)
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        100000,
    )
    .unwrap();
}

#[test]
fn memory_defaults_reject_malformed_normalized_parameter_plans() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::ExprKind as K;
    for corruption in 0..4 {
        let mut package = compile(
            "fn read(p:*bool,.q:*bool=p,.r:*bool=q)->bool {r.*} fn run() {let value=true; let result=read(value.ref);}",
        );
        let run = package
            .functions
            .iter_mut()
            .find(|f| f.name == "run")
            .unwrap();
        let K::Block { statements, .. } = &mut run.body.kind else {
            panic!("block")
        };
        let K::Let { value, .. } = &mut statements.last_mut().unwrap().kind else {
            panic!("let")
        };
        let K::Call {
            arguments,
            parameter_bindings,
            ..
        } = &mut value.kind
        else {
            panic!("call")
        };
        match corruption {
            0 => parameter_bindings[1] = parameter_bindings[0],
            1 => arguments.swap(0, 1),
            2 => arguments.swap(1, 2),
            3 => arguments[2].parameter = arguments[1].parameter,
            _ => unreachable!(),
        }
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let error = analyze_in_package(&package, run, |_| Ok(Delivery::Copy), 100000).unwrap_err();
        let diagnostic = format!("{error:?}");
        assert!(
            diagnostic.contains(match corruption {
                0 => "invalid memory default parameter frame",
                1 => "explicit memory argument follows a default",
                2 => "memory defaults are not in declaration order",
                3 => "invalid memory call argument slot",
                _ => unreachable!(),
            }),
            "{diagnostic}"
        );
    }
}

#[test]
fn memory_pointer_results_follow_parameters_fields_and_recursive_wrappers() {
    use elaboration::verification::memory;
    for body in [
        "p.value",
        "return p.value;",
        "if flag {return p.value;} q.value",
        "if flag {return p.value;} else {q.value}",
        "if flag {return p.value;} else {return q.value;}",
        "if flag {p.value} else {wrapper(true,q,p)}",
    ] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn wrapper(flag:bool,p:*Cell,q:*Cell)->*bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let targets = summary.result_targets.as_ref().unwrap();
        assert!(targets.iter().any(|p| p.parameter == 1), "{summary}");
        assert_eq!(
            targets.iter().any(|p| p.parameter == 2),
            body.contains("q"),
            "{summary}"
        );
        assert!(targets.iter().all(|p| p.fields.len() == 1), "{summary}");
    }
}

#[test]
fn memory_pointer_results_are_consumed_once_and_keep_owner_dependencies() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in [false, true] {
        for use_result in [
            "let result=selected.*;",
            "selected.* = true;",
            "let result=read(selected);",
        ] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} fn project(p:*Cell)->*bool {{p.value}} fn wrapper(p:*Cell)->*bool {{project(p)}} fn read(p:*bool)->bool {{p.*}} fn run() {{let cell=Cell {{value:true}}; let selected=wrapper(cell.ref); {} {use_result}}}",
                if moved { "let moved=cell;" } else { "" }
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(result.is_ok(), !moved, "{result:?}");
        }
    }
    for operation in [
        "let result=identity(cell.ref).value.*;",
        "identity(cell.ref).value.* = false;",
        "let result=read(identity(cell.value.ref));",
        "var p=cell.value.ref; p=identity(p); let result=p.*;",
    ] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn identity(p:*T)->*T where T {{p}} fn read(p:*bool)->bool {{p.*}} fn run() {{let cell=Cell {{value:true}}; {operation}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        )
        .unwrap();
    }
}

#[test]
fn memory_pointer_results_reject_local_escape_and_keep_loaded_paths() {
    use elaboration::verification::memory;
    let package = compile("fn escape()->*bool {let local=true; local.ref}");
    assert!(
        memory::infer_in_package(&package, package.functions[0].declaration, &[], &mut 100000)
            .is_err()
    );
    let package = compile(
        "fn load(slot:*(*bool))->*bool {slot.*} fn wrapper(slot:*(*bool))->*bool {load(slot)}",
    );
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
    let targets = summary.result_targets.as_ref().unwrap();
    assert_eq!(targets.len(), 1);
    assert!(
        targets
            .iter()
            .all(|t| t.parameter == 0 && t.through == vec![vec![]] && t.fields.is_empty())
    );
    assert_eq!(summary.accesses.len(), 1, "{summary}");
}

#[test]
fn memory_pointer_results_snapshot_arguments_before_later_reassignment() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=first;", "let moved=second;"] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn select(p:*bool,flag:bool)->*bool {{p}} fn run() {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; var p=first.value.ref; let selected=select(p,do {{p=second.value.ref; true}}); {moved} let result=selected.*;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(
            result.is_ok(),
            moved != "let moved=first;",
            "{moved}: {result:?}"
        );
    }
}

#[test]
fn memory_pointer_results_cross_blocks_branches_without_reviving_retired_storage() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (expression, valid) in [
        ("do {let alias=identity(first.value.ref); alias}", true),
        (
            "if flag {identity(first.value.ref)} else {identity(second.value.ref)}",
            true,
        ),
        (
            "do {let local=Cell {value:true}; identity(local.value.ref)}",
            false,
        ),
        (
            "if flag {identity(first.value.ref)} else {do {let local=Cell {value:true}; identity(local.value.ref)}}",
            false,
        ),
    ] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn identity(p:*bool)->*bool {{p}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let selected={expression}; let result=selected.*;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{expression}: {result:?}");
    }
}

#[test]
fn memory_rebinding_tracks_strong_updates_branches_and_short_circuit_paths() {
    use elaboration::verification::memory;
    for (body, expected) in [
        ("p=q; p.*", vec![1]),
        ("if flag {p=q;} p.*", vec![0, 1]),
        ("if flag {p=q;} else {p=q;} p.*", vec![1]),
        ("let used=flag and do {p=q; true}; p.*", vec![0, 1]),
        ("let used=flag or do {p=q; false}; p.*", vec![0, 1]),
        ("p=q; let selected=p; p=r; selected.*", vec![1]),
    ] {
        let package = compile(&format!(
            "fn inspect(p:*bool,q:*bool,r:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let actual: std::collections::BTreeSet<_> = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect();
        assert_eq!(actual, expected.into_iter().collect(), "{body}: {summary}");
    }
}

#[test]
fn memory_rebinding_loop_fixed_point_includes_backedges_and_early_exits() {
    use elaboration::verification::memory;
    for body in [
        "while flag {let seen=p.*; p=q;} p.*",
        "while flag {if flag {p=q; break;} p=r;} p.*",
        "while flag {let seen=p.*; p=q; if flag {continue;} p=r;} p.*",
        "while flag {while flag {p=q;} let seen=p.*; p=r;} p.*",
    ] {
        let package = compile(&format!(
            "fn inspect(p:*bool,q:*bool,r:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let actual: std::collections::BTreeSet<_> = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect();
        assert!(
            actual.contains(&0) && actual.contains(&1),
            "{body}: {summary}"
        );
        if body.contains("p=r") {
            assert!(actual.contains(&2), "{body}: {summary}");
        }
    }
}

#[test]
fn memory_rebinding_preserves_indirect_alternatives() {
    use elaboration::verification::memory;
    for (body, accepted) in [
        ("var p=slot.*; if flag {p=known;} p.*", false),
        ("var p=slot.*; if flag {p=known;} else {p=known;} p.*", true),
        ("var p=slot.*; p=known; p.*", true),
        ("var p=known; if flag {p=slot.*;} p.*", false),
        ("var p=known; while flag {p=slot.*;} p.*", false),
    ] {
        let package = compile(&format!(
            "fn inspect(slot:*(*bool),known:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let result = memory::infer_in_package(&package, function.declaration, &[], &mut 100000);
        let summary = result.unwrap();
        assert_eq!(
            summary
                .accesses
                .iter()
                .any(|a| !a.target.through.is_empty()),
            !accepted,
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_rebinding_call_contract_checks_the_replacement_pointee() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=first;", "let moved=second;"] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn read(p:*bool,q:*bool)->bool {{p=q; p.*}} fn run() {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let p=first.value.ref; let q=second.value.ref; {moved} let result=read(p,q);}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(
            result.is_ok(),
            moved != "let moved=second;",
            "{moved}: {result:?}"
        );
    }
}

#[test]
fn memory_rebinding_loop_growth_exhausts_without_publishing_partial_origins() {
    use elaboration::verification::memory;
    let count = 12;
    let parameters = (0..count)
        .map(|i| format!("p{i}:*bool"))
        .collect::<Vec<_>>()
        .join(",");
    let mut body = (0..count - 1)
        .map(|i| format!("p{i}=p{};", i + 1))
        .collect::<String>();
    body.push_str(&format!("p{}=p0;", count - 1));
    let package = compile(&format!(
        "fn rotate({parameters},flag:bool)->*bool {{while flag {{{body}}} p0}}"
    ));
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "rotate")
        .unwrap();
    let error =
        memory::infer_in_package(&package, function.declaration, &[], &mut 1000).unwrap_err();
    assert!(error.contains("budget"), "{error}");
    // Retry the same body with sufficient budget; no partial result survives
    // the earlier failed fixed-point transaction.
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 1000000).unwrap();
    let targets = summary.result_targets.unwrap();
    assert_eq!(targets.len(), count);
    assert_eq!(
        targets.iter().map(|p| p.parameter).collect::<Vec<_>>(),
        (0..count).collect::<Vec<_>>()
    );
}

#[test]
fn pointer_slots_exact_writes_replace_origins_and_preserve_existing_copies() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for selected in ["p", "copy", "slot.*"] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} fn run() {{let first=Cell {{value:false}}; let second=Cell {{value:true}}; var p=first.value.ref; let slot=p.ref; let copy=slot.*; slot.* = second.value.ref; {moved} let seen=({selected}).*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            let accepted = if selected == "copy" {
                moved != "let moved=first;"
            } else {
                moved != "let moved=second;"
            };
            assert_eq!(result.is_ok(), accepted, "{selected}, {moved}: {result:?}");
        }
    }
}

#[test]
fn pointer_slots_field_writes_are_visible_through_local_and_indirect_reads() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for operation in [
        "holder.ptr=second.value.ref;",
        "holder.ptr.ref.* = second.value.ref;",
    ] {
        for read in ["holder.ptr.*", "holder.ptr.ref.*.*"] {
            for moved in ["", "let moved=first;", "let moved=second;"] {
                let package = compile(&format!(
                    "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} fn run() {{let first=Cell {{value:false}}; let second=Cell {{value:true}}; let holder=Holder {{ptr:first.value.ref}}; {operation} {moved} let seen={read};}}"
                ));
                let run = package.functions.iter().find(|f| f.name == "run").unwrap();
                let result = analyze_in_package(
                    &package,
                    run,
                    |ty| {
                        Ok(
                            if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                                Delivery::Copy
                            } else {
                                Delivery::Move
                            },
                        )
                    },
                    100000,
                );
                assert_eq!(
                    result.is_ok(),
                    moved != "let moved=second;",
                    "{operation}, {read}, {moved}: {result:?}"
                );
            }
        }
    }
}

#[test]
fn pointer_slots_snapshot_destination_before_rhs_retargets_the_slot_pointer() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (selected, accepted) in [("p", true), ("q", false)] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn run() {{let first=Cell {{value:false}}; let second=Cell {{value:true}}; var p=first.value.ref; var q=first.value.ref; var slot=p.ref; slot.* = do {{slot=q.ref; second.value.ref}}; let moved=first; let seen={selected}.*;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), accepted, "{selected}: {result:?}");
    }
}

#[test]
fn pointer_slots_unknown_weak_writes_and_parent_replacement_keep_no_stale_origin() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    // An opaque entry pointer is not made known by a merely possible write.
    for source in [
        "struct Holder {ptr:*bool,} fn run(flag:bool,unknown:*bool) {let first=false; let second=true; let holder=Holder {ptr:unknown}; var p=first.ref; let slot=if flag {holder.ptr.ref} else {p.ref}; slot.* = second.ref; let seen=holder.ptr.*;}",
        "struct Cell {value:bool,} struct Holder {ptr:*bool,} fn run() {let first=Cell {value:false}; let second=Cell {value:true}; var holder=Holder {ptr:first.value.ref}; holder.ptr=first.value.ref; holder=Holder {ptr:second.value.ref}; let moved=second; let seen=holder.ptr.*;}",
        "fn run() {let outer=false; var p=outer.ref; let slot=p.ref; do {let inner=true; slot.* = inner.ref;}; let seen=slot.*.*;}",
    ] {
        let package = compile(source);
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert!(result.is_err(), "{source}: {result:?}");
    }
}

#[test]
fn aggregate_pointer_origins_follow_construction_nested_moves_and_replacement() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for construction in [
        "let holder=Holder {ptr:first.value.ref};",
        "let original=Holder {ptr:first.value.ref}; let holder=original;",
        "let nested=Outer {inner:Holder {ptr:first.value.ref}}; let holder=nested.inner;",
        "let holder=do {let local=Holder {ptr:first.value.ref}; local};",
        "var holder=Holder {ptr:second.value.ref}; holder=Holder {ptr:first.value.ref};",
        "var holder=Holder {ptr:first.value.ref}; holder=holder;",
        "var outer=Outer {inner:Holder {ptr:second.value.ref}}; outer.inner=Holder {ptr:first.value.ref}; let holder=outer.inner;",
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} struct Outer {{inner:Holder,}} fn run() {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; {construction} {moved} let seen=holder.ptr.*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                moved != "let moved=first;",
                "{construction}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn aggregate_pointer_origins_preserve_branch_alternatives_and_scope_retirement() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (expression, valid) in [
        (
            "if flag {Holder {ptr:first.value.ref}} else {Holder {ptr:second.value.ref}}",
            true,
        ),
        (
            "do {let local=Cell {value:true}; Holder {ptr:local.value.ref}}",
            false,
        ),
        (
            "if flag {Holder {ptr:first.value.ref}} else {do {let local=Cell {value:true}; Holder {ptr:local.value.ref}}}",
            false,
        ),
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let holder={expression}; {moved} let seen=holder.ptr.*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                valid && moved.is_empty(),
                "{expression}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn aggregate_pointer_origins_capture_each_field_before_later_field_effects() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    // Struct initialization follows declaration order, independently of the
    // order of explicitly named inputs (docs/type/user-defined-types/structs.md).
    for (declaration, fields, dependency) in [
        (
            "ptr:*bool,mark:bool,",
            "ptr:p,mark:do {p=second.value.ref; true}",
            "first",
        ),
        (
            "ptr:*bool,mark:bool,",
            "mark:do {p=second.value.ref; true},ptr:p",
            "first",
        ),
        (
            "mark:bool,ptr:*bool,",
            "ptr:p,mark:do {p=second.value.ref; true}",
            "second",
        ),
    ] {
        for moved in ["first", "second"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} struct Holder {{{declaration}}} fn run() {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; var p=first.value.ref; let holder=Holder {{{fields}}}; let moved={moved}; let seen=holder.ptr.*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                moved != dependency,
                "{fields}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn pattern_pointer_origins_follow_nested_bindings_and_owned_subtrees() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for setup in [
        "let {ptr:p}=Holder {ptr:first.value.ref};",
        "let (p,_)=((first.value.ref),second.value.ref);",
        "let {pair:(p,_)}=Outer {pair:(first.value.ref,second.value.ref)};",
        "let {inner:p}=Nested {inner:Holder {ptr:first.value.ref}};",
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let access = if setup.contains("inner:p") {
                "p.ptr.*"
            } else {
                "p.*"
            };
            let package = tuple_support::compile_with_tuple(&format!(
                "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} struct Outer {{pair:Tuple<*bool,*bool>,}} struct Nested {{inner:Holder,}} fn run() {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; {setup} {moved} let seen={access};}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                moved != "let moved=first;",
                "{setup}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn observation_pattern_whole_bindings_preserve_pointer_dependencies() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in [false, true] {
        let invalidate = if moved { "let moved=cell;" } else { "" };
        let package = compile(&format!(
            "struct Cell {{value:bool}} struct Holder {{flag:bool,ptr:*bool}}
             fn run() {{let cell=Cell {{value:true}};
                let holder=Holder {{flag:true,ptr:cell.value.ref}};
                let ({{flag:true}} or {{flag:false}}) as whole=holder else {{return;}};
                {invalidate} let seen=whole.ptr.*;
             }}"
        ));
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap();
        let result = analyze_in_package(
            &package,
            run,
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
        );
        assert_eq!(result.is_ok(), !moved, "moved={moved}: {result:?}");
    }
}

#[test]
fn bound_or_patterns_preserve_canonical_pointer_binding_origins() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in [false, true] {
        let invalidate = if moved { "let moved=cell;" } else { "" };
        let package = compile(&format!(
            "struct Cell {{value:bool}} struct Holder {{flag:bool,ptr:*bool}}
             fn run(flag:bool) {{let cell=Cell {{value:true}};
                let holder=Holder {{flag:flag,ptr:cell.value.ref}};
                let ({{flag:true,ptr:p}} or {{flag:false,ptr:p}})=holder else {{return;}};
                {invalidate} let seen=p.*;
             }}"
        ));
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap();
        let result = analyze_in_package(
            &package,
            run,
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
        );
        assert_eq!(result.is_ok(), !moved, "moved={moved}: {result:?}");
    }
}

#[test]
fn overlapping_copy_patterns_preserve_pointer_dependencies_in_both_owners() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for access in ["copied.*", "whole.ptr.*"] {
        for moved in [false, true] {
            let invalidate = if moved { "let moved=cell;" } else { "" };
            let package = compile(&format!(
                "struct Cell {{value:bool}} struct Holder {{ptr:*bool}}
             fn run() {{let cell=Cell {{value:true}};
                let holder=Holder {{ptr:cell.value.ref}};
                let {{ptr:copied}} as whole=holder;
                {invalidate} let seen={access};
             }}"
            ));
            let run = package
                .functions
                .iter()
                .find(|function| function.name == "run")
                .unwrap();
            let result = analyze_in_package(
                &package,
                run,
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
            );
            assert_eq!(
                result.is_ok(),
                !moved,
                "access={access}, moved={moved}: {result:?}"
            );
        }
    }
}

#[test]
fn conditional_copy_patterns_merge_all_possible_pointer_targets() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=first;", "let moved=second;"] {
        let package = compile(&format!(
            "struct Cell {{value:bool}}
             fn run(flag:bool) {{let first=Cell {{value:true}};let second=Cell {{value:false}};
                let ((true,p,_) or (false,_,p)) as whole=(flag,first.value.ref,second.value.ref);
                {moved} let seen=p.*;
             }}"
        ));
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap();
        let result = analyze_in_package(
            &package,
            run,
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
        );
        assert_eq!(result.is_ok(), moved.is_empty(), "{moved}: {result:?}");
    }
}

#[test]
fn ordinary_or_copy_patterns_merge_pointer_targets_without_whole_binding() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=first;", "let moved=second;"] {
        let package = compile(&format!(
            "struct Cell {{value:bool}}
             fn run(flag:bool) {{let first=Cell {{value:true}};let second=Cell {{value:false}};
                let ((true,p,_) or (false,_,p))=(flag,first.value.ref,second.value.ref);
                {moved} let seen=p.*;
             }}"
        ));
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap();
        let result = analyze_in_package(
            &package,
            run,
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
        );
        assert_eq!(result.is_ok(), moved.is_empty(), "{moved}: {result:?}");
    }
}

#[test]
fn conditional_affine_patterns_merge_pointer_origins_of_moved_holders() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=first;", "let moved=second;"] {
        let package=compile(&format!(
            "struct Cell {{value:bool}} struct Holder {{ptr:*bool}}
             fn run(flag:bool) {{let first=Cell {{value:true}};let second=Cell {{value:false}};
                let ((true,item,_) or (false,_,item))=(flag,Holder {{ptr:first.value.ref}},Holder {{ptr:second.value.ref}});
                {moved} let seen=item.ptr.*;
             }}"
        ));
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap();
        let result = analyze_in_package(
            &package,
            run,
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
        );
        assert_eq!(result.is_ok(), moved.is_empty(), "{moved}: {result:?}");
    }
}

#[test]
fn pattern_pointer_origins_reach_match_arms_and_let_else_success() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for operation in [
        "holder match {{ptr:p}=>{let seen=p.*;}}",
        "let {ptr:p}=holder; let seen=p.*;",
        "let (p,true)=(holder.ptr,flag) else {return;}; let seen=p.*;",
        "let choice=Choice.some(holder.ptr); choice match {.some(p)=>{let seen=p.*;},.none=>()}",
    ] {
        for moved in [false, true] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} enum Choice {{none,some(*bool)}} fn run(flag:bool) {{let cell=Cell {{value:true}}; let holder=Holder {{ptr:cell.value.ref}}; {} {operation}}}",
                if moved { "let moved=cell;" } else { "" }
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(result.is_ok(), !moved, "{operation}, {moved}: {result:?}");
        }
    }
}

#[test]
fn pattern_pointer_guards_observe_pointees_without_leaking_failed_candidates() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (guard, moved, accepted) in [
        ("p.*", false, true),
        ("p.*", true, false),
        ("false and p.*", true, true),
    ] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} fn run() {{let cell=Cell {{value:true}}; let holder=Holder {{ptr:cell.value.ref}}; {} holder match {{{{ptr:p}} if {guard} =>(),_=>()}}}}",
            if moved { "let moved=cell;" } else { "" }
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), accepted, "{guard}, {moved}: {result:?}");
    }
    let package = compile(
        "struct Cell {value:bool,} struct Holder {ptr:*bool,} fn run() {let cell=Cell {value:true}; let holder=Holder {ptr:cell.value.ref}; holder match {{ptr:failed} if false=>{let moved=cell; let unseen=failed.*;},{ptr:accepted}=>{let seen=accepted.*;}}}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        100000,
    )
    .unwrap();
}

#[test]
fn pattern_pointer_origins_merge_enum_candidates_without_forgetting_dependencies() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=first;", "let moved=second;"] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} enum Choice {{left(*bool),right(*bool)}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let choice=if flag {{Choice.left(first.value.ref)}} else {{Choice.right(second.value.ref)}}; {moved} choice match {{.left(p)=>{{let seen=p.*;}},.right(p)=>{{let seen=p.*;}}}}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), moved.is_empty(), "{moved}: {result:?}");
    }
}

#[test]
fn match_result_origins_join_live_pointer_and_aggregate_arms() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (expression, access) in [
        (
            "flag match {true=>first.value.ref,false=>second.value.ref}",
            "selected.*",
        ),
        (
            "flag match {true=>Holder {ptr:first.value.ref},false=>Holder {ptr:second.value.ref}}",
            "selected.ptr.*",
        ),
        (
            "Holder {ptr:first.value.ref} match {{ptr:p}=>p}",
            "selected.*",
        ),
        (
            "Holder {ptr:first.value.ref} match {whole=>whole}",
            "selected.ptr.*",
        ),
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let selected={expression}; {moved} let seen={access};}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            let accepted =
                moved.is_empty() || moved == "let moved=second;" && !expression.contains("second");
            assert_eq!(
                result.is_ok(),
                accepted,
                "{expression}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn match_result_origins_exclude_diverging_and_failed_guard_paths() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for expression in [
        "flag match {true=>first.value.ref,false=>{return;}}",
        "flag match {_ if false=>second.value.ref,_=>first.value.ref}",
        "true match {true=>first.value.ref,false=>second.value.ref}",
    ] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let selected={expression}; let moved=second; let seen=selected.*;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        )
        .unwrap();
    }
}

#[test]
fn match_result_origins_do_not_escape_pattern_bindings_or_arm_locals() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for expression in [
        "Holder {ptr:value.ref} match {{ptr:p}=>p.ref}",
        "flag match {true=>do {let local=value.ref; local.ref},false=>p.ref}",
    ] {
        let package = compile(&format!(
            "struct Holder {{ptr:*bool,}} fn run(flag:bool) {{let value=true; let p=value.ref; let escaped={expression}; let seen=escaped.*.*;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        assert!(
            analyze_in_package(
                &package,
                run,
                |ty| Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    }
                ),
                100000
            )
            .is_err(),
            "{expression}"
        );
    }
}

#[test]
fn match_result_origins_feed_pointer_arguments_and_direct_dereference() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for operation in [
        "let seen=(flag match {true=>first.ref,false=>second.ref}).*;",
        "let seen=read(flag match {true=>first.ref,false=>second.ref});",
        "var p=first.ref; p=flag match {true=>first.ref,false=>second.ref}; let seen=p.*;",
    ] {
        let package = compile(&format!(
            "fn read(p:*bool)->bool {{p.*}} fn run(flag:bool) {{let first=true; let second=false; {operation}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        analyze_in_package(&package, run, |_| Ok(Delivery::Copy), 100000).unwrap();
    }
}

#[test]
fn match_result_origins_preserve_unknown_alternatives() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for result in [
        "let selected=flag match {true=>value.ref,false=>unknown}; let seen=selected.*;",
        "let selected=flag match {true=>Holder {ptr:value.ref},false=>Holder {ptr:unknown}}; let seen=selected.ptr.*;",
    ] {
        let package = compile(&format!(
            "struct Holder {{ptr:*bool,}} fn run(flag:bool,unknown:*bool) {{let value=true; {result}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert!(result.is_err(), "{result:?}");
    }
}

#[test]
fn temporary_projection_origins_preserve_nested_and_match_result_dependencies() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for expression in [
        "(Holder {ptr:first.value.ref}).ptr",
        "(Outer {inner:Holder {ptr:first.value.ref},rest:Ticket {}}).inner.ptr",
        "(do {let local=Holder {ptr:first.value.ref}; local}).ptr",
        "(flag match {true=>Holder {ptr:first.value.ref},false=>Holder {ptr:second.value.ref}}).ptr",
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} struct Holder {{ptr:*bool,}} struct Ticket {{}} struct Outer {{inner:Holder,rest:Ticket,}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let p={expression}; {moved} let seen=p.*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            let accepted =
                moved.is_empty() || moved == "let moved=second;" && !expression.contains("second");
            assert_eq!(
                result.is_ok(),
                accepted,
                "{expression}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn temporary_projection_origins_transfer_aggregate_fields_and_keep_residual_cleanup() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Ticket {} struct Holder {ptr:*bool,} struct Outer {inner:Holder,rest:Ticket,} fn run() {let value=true; let picked=(Outer {inner:Holder {ptr:value.ref},rest:Ticket {}}).inner; let seen=picked.ptr.*;}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        100000,
    )
    .unwrap();
    assert_eq!(report.temporary_owners.len(), 1, "{report}");
    let remaining = report.temporary_owners[0].remaining.remaining();
    assert!(remaining.iter().any(|(p, _)| p.fields == [1]), "{report}");
    assert!(!remaining.iter().any(|(p, _)| p.fields == [0]), "{report}");
}

#[test]
fn temporary_projection_origins_do_not_extend_pointee_scope() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Holder {ptr:*bool,} fn run() {let p=(do {let local=true; Holder {ptr:local.ref}}).ptr; let seen=p.*;}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    assert!(
        analyze_in_package(
            &package,
            run,
            |ty| Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                }
            ),
            100000
        )
        .is_err()
    );
}

#[test]
fn memory_match_summaries_compose_result_origins_and_later_rebindings() {
    use elaboration::verification::memory;
    for body in [
        "flag match {true=>p,false=>q}",
        "flag match {true=>{return p;},false=>q}",
        "flag match {_ if flag=>p,_=>q}",
        "var selected=p; flag match {true=>{selected=q;},false=>()}; selected",
        "flag match {true=>p,false=>select(true,q,p)}",
    ] {
        let package = compile(&format!(
            "fn select(flag:bool,p:*bool,q:*bool)->*bool {{{body}}} fn wrapper(flag:bool,p:*bool,q:*bool)->bool {{select(flag,p,q).*}}"
        ));
        let select = package
            .functions
            .iter()
            .find(|f| f.name == "select")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, select.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .map(|p| p.parameter)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "{body}: {summary}"
        );
        let wrapper = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
        let reads: std::collections::BTreeSet<_> = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect();
        assert_eq!(reads, [1, 2].into_iter().collect(), "{body}: {summary}");
    }
}

#[test]
fn memory_match_summaries_keep_guard_reads_and_skip_proven_impossible_bodies() {
    use elaboration::verification::memory;
    for (body, expected) in [
        ("p match {bound if bound.* =>q,_=>q}", vec![0]),
        (
            "true match {false=>{let local=true; local.ref},true=>p}",
            vec![],
        ),
        ("p match {_ if false and p.* =>q,bound=>bound}", vec![]),
        ("p match {_ if p.* and false=>q,_=>q}", vec![0]),
        ("p match {bound or bound=>bound}", vec![]),
    ] {
        let package = compile(&format!("fn select(p:*bool,q:*bool)->*bool {{{body}}}"));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "select")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let reads: std::collections::BTreeSet<_> = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect();
        assert_eq!(reads, expected.into_iter().collect(), "{body}: {summary}");
    }
}

#[test]
fn memory_match_call_sites_check_every_returned_owner_and_guard_access() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for body in [
        "flag match {true=>p,false=>q}",
        "p match {_ if p.* =>q,_=>q}",
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} fn select(flag:bool,p:*bool,q:*bool)->*bool {{{body}}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let p=first.value.ref; let q=second.value.ref; {moved} let seen=select(flag,p,q).*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                moved.is_empty(),
                "{body}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn memory_match_summaries_describe_writes_without_authorizing_guard_effects() {
    use elaboration::verification::memory;
    let package = compile(
        "fn select(flag:bool,p:*bool)->*bool {flag match {(true or false) as seen if seen=>p,_=>p}}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "select")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert_eq!(
        summary
            .result_targets
            .unwrap()
            .iter()
            .map(|p| p.parameter)
            .collect::<Vec<_>>(),
        vec![1]
    );
    let package = compile(
        "fn select(p:*bool,q:*bool)->*bool {p match {_ if do {p.* = false; true}=>p,_=>q}}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "select")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.write),
        "{summary}"
    );
    // A descriptive write footprint is not permission for an observing guard.
    let error = elaboration::verification::moves::analyze_in_package_with_borrows(
        &package,
        function,
        |_| Ok(elaboration::verification::moves::Delivery::Copy),
        100000,
        None,
        &[
            (0, nir::Term::Primitive(nir::Primitive::Bool)),
            (1, nir::Term::Primitive(nir::Primitive::Bool)),
        ],
    )
    .unwrap_err();
    assert!(error.reason.contains("observation"), "{error}");
}

#[test]
fn memory_conditionals_capture_results_before_joining_branch_environments() {
    use elaboration::verification::memory;
    for body in [
        "var selected=q; if flag {selected=p; selected} else {p}",
        "var selected=p; if flag {selected=q; return p;} selected",
        "if true {p} else {let local=true; local.ref}",
        "if false {q} else {p}",
    ] {
        let package = compile(&format!(
            "fn select(flag:bool,p:*bool,q:*bool)->*bool {{{body}}} fn read(flag:bool,p:*bool,q:*bool)->bool {{select(flag,p,q).*}}"
        ));
        let select = package
            .functions
            .iter()
            .find(|f| f.name == "select")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, select.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .map(|p| p.parameter)
                .collect::<Vec<_>>(),
            vec![1],
            "{body}: {summary}"
        );
        let read = package.functions.iter().find(|f| f.name == "read").unwrap();
        let summary =
            memory::infer_in_package(&package, read.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .accesses
                .iter()
                .filter(|a| a.read_value)
                .map(|a| a.target.parameter)
                .collect::<Vec<_>>(),
            vec![1],
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_conditionals_respect_short_circuit_accesses_and_rebindings() {
    use elaboration::verification::memory;
    for (expression, expected) in [
        ("false and q.*", vec![]),
        ("true or q.*", vec![]),
        ("true and q.*", vec![2]),
        ("false or q.*", vec![2]),
        ("flag and q.*", vec![2]),
        ("flag or q.*", vec![2]),
        ("q.* and false", vec![2]),
    ] {
        let package = compile(&format!(
            "fn select(flag:bool,p:*bool,q:*bool)->*bool {{let ignored={expression}; p}}"
        ));
        let select = package
            .functions
            .iter()
            .find(|f| f.name == "select")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, select.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .accesses
                .iter()
                .filter(|a| a.read_value)
                .map(|a| a.target.parameter)
                .collect::<Vec<_>>(),
            expected,
            "{expression}: {summary}"
        );
    }
    for (condition, expected) in [
        ("true and", vec![2]),
        ("false or", vec![2]),
        ("false and", vec![1]),
        ("true or", vec![1]),
        ("flag and", vec![1, 2]),
    ] {
        let package = compile(&format!(
            "fn select(flag:bool,p:*bool,q:*bool)->*bool {{var chosen=p; let ignored={condition} do {{chosen=q; true}}; chosen}}"
        ));
        let select = package
            .functions
            .iter()
            .find(|f| f.name == "select")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, select.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .result_targets
                .unwrap()
                .iter()
                .map(|p| p.parameter)
                .collect::<Vec<_>>(),
            expected,
            "{condition}"
        );
    }
}

#[test]
fn memory_conditionals_call_sites_validate_only_reachable_accesses() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for body in [
        "var selected=q; if flag {selected=p; selected} else {p}",
        "var selected=p; if flag {selected=q; return p;} selected",
        "let ignored=false and q.*; p",
        "let ignored=true or q.*; p",
    ] {
        for (moved, valid) in [
            ("", true),
            ("let moved=second;", true),
            ("let moved=first;", false),
        ] {
            let package = compile(&format!(
                "struct Cell {{value:bool,}} fn select(flag:bool,p:*bool,q:*bool)->*bool {{{body}}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let p=first.value.ref; let q=second.value.ref; {moved} let seen=select(flag,p,q).*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(result.is_ok(), valid, "{body}, {moved}: {result:?}");
        }
    }
    for body in [
        "let ignored=q.* and false; p",
        "let ignored=flag and q.*; p",
    ] {
        let package = compile(&format!(
            "struct Cell {{value:bool,}} fn select(flag:bool,p:*bool,q:*bool)->*bool {{{body}}} fn run(flag:bool) {{let first=Cell {{value:true}}; let second=Cell {{value:false}}; let p=first.value.ref; let q=second.value.ref; let moved=second; let seen=select(flag,p,q).*;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        assert!(
            analyze_in_package(
                &package,
                run,
                |ty| Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    }
                ),
                100000
            )
            .is_err(),
            "{body}"
        );
    }
}

#[test]
fn bitvec_memory_summaries_preserve_operand_reads_and_eager_rebindings() {
    use elaboration::verification::memory;
    for body in [
        "((p.*)'bitvec ++ (q.*)'bitvec)[4..12].as(.u8'builtin)",
        "((p.*)'bitvec and (q.*)'bitvec).as(.u8'builtin)",
        "((p.*)'bitvec or (q.*)'bitvec).as(.u8'builtin)",
        "let same=(p.*)'bitvec == (q.*)'bitvec; (not (p.*)'bitvec).as(.u8'builtin)",
        "((p.*)'bitvec[0..0] ++ (q.*)'bitvec).as(.u8'builtin)",
    ] {
        let package = compile(&format!(
            "fn combine(p:*.u8'builtin,q:*.u8'builtin)->.u8'builtin {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "combine")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let reads: std::collections::BTreeSet<_> = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect();
        assert_eq!(reads, [0, 1].into_iter().collect(), "{body}: {summary}");
    }
    for operator in ["and", "or"] {
        let package = compile(&format!(
            "fn select(p:*.u8'builtin,q:*.u8'builtin)->*.u8'builtin {{var chosen=p; let bits:.u8'builtin=0; let ignored=bits'bitvec {operator} do {{chosen=q; bits'bitvec}}; chosen}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "select")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .result_targets
                .unwrap()
                .iter()
                .map(|p| p.parameter)
                .collect::<Vec<_>>(),
            vec![1],
            "{operator}"
        );
    }
}

#[test]
fn bitvec_resource_analysis_checks_reads_including_zero_width_inputs() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for expression in [
        "((p.*)'bitvec ++ (q.*)'bitvec)[4..12].as(.u8'builtin)",
        "((p.*)'bitvec and (q.*)'bitvec).as(.u8'builtin)",
        "((p.*)'bitvec or (q.*)'bitvec).as(.u8'builtin)",
        "(p.*)'bitvec == (q.*)'bitvec",
        "((p.*)'bitvec[0..0] ++ (q.*)'bitvec).as(.u8'builtin)",
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:.u8'builtin,}} fn run() {{let first=Cell {{value:0}}; let second=Cell {{value:255}}; let p=first.value.ref; let q=second.value.ref; {moved} let result={expression};}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(
                result.is_ok(),
                moved.is_empty(),
                "{expression}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn bitvec_eager_rhs_keeps_definite_pointer_updates_in_resource_analysis() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for operator in ["and", "or"] {
        for (moved, valid) in [("let moved=first;", true), ("let moved=second;", false)] {
            let package = compile(&format!(
                "struct Cell {{value:.u8'builtin,}} fn run() {{let first=Cell {{value:0}}; let second=Cell {{value:255}}; var chosen=first.value.ref; let q=second.value.ref; let zero:.u8'builtin=0; let ignored=zero'bitvec {operator} do {{chosen=q; zero'bitvec}}; {moved} let seen=chosen.*;}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let result = analyze_in_package(
                &package,
                run,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                100000,
            );
            assert_eq!(result.is_ok(), valid, "{operator}, {moved}: {result:?}");
        }
    }
}

#[test]
fn bit_writeback_resource_analysis_preserves_initialized_places_and_rhs_order() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        (
            "packet.word'bitvec[4..12] = packet.word'bitvec[0..8];",
            true,
        ),
        (
            "packet.word'bitvec[0..8] = do {packet.word=43981; packet.other=99; 18};",
            true,
        ),
        ("let moved=packet; packet.word'bitvec[0..8] = 0;", false),
        (
            "packet.word'bitvec[0..8] = do {let moved=packet; 0};",
            false,
        ),
        (
            "packet.word'bitvec[0..8] = do {let moved=packet; packet=Packet {word:0,other:0}; 0};",
            true,
        ),
        (
            "packet.word'bitvec[0..8] = do {let moved=packet; return;};",
            true,
        ),
        ("let moved=packet; packet.word'bitvec[16..16] = 0;", false),
        ("packet.word'bitvec[16..16] = 0;", true),
    ] {
        let package = compile(&format!(
            "typealias Word=.u16'builtin; struct Packet {{word:Word,other:Word,}} fn run() {{var packet=Packet {{word:4660,other:0}}; {body}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn bit_writeback_resource_analysis_keeps_pointer_identity_and_checks_rhs_reads() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("packet.word'bitvec[0..8] = 18; let seen=p.*;", true),
        (
            "let moved=packet; other.word'bitvec[0..8] = (p.*)'bitvec[0..8];",
            false,
        ),
    ] {
        let package = compile(&format!(
            "typealias Word=.u16'builtin; struct Packet {{word:Word,}} fn run() {{let packet=Packet {{word:4660}}; let other=Packet {{word:0}}; let p=packet.word.ref; {body}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn bit_writeback_resource_analysis_rejects_malformed_place_and_width_metadata() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::ExprKind as K;
    let package = compile(
        "typealias Word=.u16'builtin; struct Packet {word:Word,} fn run() {let packet=Packet {word:0}; packet.word'bitvec[0..8] = 1;}",
    );
    for corruption in 0..6 {
        let mut function = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .clone();
        let K::Block { statements, result } = &mut function.body.kind else {
            panic!("block")
        };
        let assignment = statements
            .iter_mut()
            .chain(result.as_deref_mut())
            .find(|e| matches!(e.kind, K::BitAssign { .. }))
            .expect("bit assignment");
        let K::BitAssign {
            place,
            storage_type,
            start,
            end,
            value,
        } = &mut assignment.kind
        else {
            unreachable!()
        };
        match corruption {
            0 => *start = 9,
            1 => *end = 17,
            2 => *storage_type = nir::Term::Primitive(nir::Primitive::Bool),
            3 => place.fields[0].1 = 99,
            4 => place.fields[0].0 = nir::program::FieldOwner::Nominal(function.declaration),
            5 => value.ty = nir::Term::Primitive(nir::Primitive::BitVec(7)),
            _ => unreachable!(),
        }
        assert!(
            analyze_in_package(
                &package,
                &function,
                |ty| Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    }
                ),
                100000
            )
            .is_err(),
            "corruption {corruption}"
        );
    }
}

#[test]
fn bit_writeback_memory_summaries_keep_rhs_accesses_and_local_updates_private() {
    use elaboration::verification::memory;
    for body in [
        "var word=p.*; word'bitvec[4..12] = (q.*)'bitvec[0..8]; word",
        "var word=p.*; word'bitvec[0..8] = do {word=43981; (q.*)'bitvec[0..8]}; word",
        "var word=p.*; word'bitvec[16..16] = (q.*)'bitvec[0..0]; word",
        "p.* match {word=>{word'bitvec[4..12] = (q.*)'bitvec[0..8]; word}}",
    ] {
        let package = compile(&format!(
            "typealias Word=.u16'builtin; fn edit(p:*Word,q:*Word)->Word {{{body}}} fn wrapper(p:*Word,q:*Word)->Word {{edit(p,q)}}"
        ));
        for name in ["edit", "wrapper"] {
            let function = package.functions.iter().find(|f| f.name == name).unwrap();
            let summary =
                memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
            assert_eq!(
                summary
                    .accesses
                    .iter()
                    .filter(|a| a.read_value)
                    .map(|a| a.target.parameter)
                    .collect::<Vec<_>>(),
                vec![0, 1],
                "{body}: {summary}"
            );
            assert!(
                summary.accesses.iter().all(|a| !a.write),
                "local write escaped into footprint: {summary}"
            );
        }
    }
}

#[test]
fn bit_writeback_helper_calls_check_pointees_without_modifying_their_ownership() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=first;", "let moved=second;"] {
        let package = compile(&format!(
            "typealias Word=.u16'builtin; struct Cell {{value:Word,}} fn edit(p:*Word,q:*Word)->Word {{var word=p.*; word'bitvec[4..12] = (q.*)'bitvec[0..8]; word}} fn run() {{let first=Cell {{value:4660}}; let second=Cell {{value:171}}; let p=first.value.ref; let q=second.value.ref; {moved} let result=edit(p,q); let still=p.*;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), moved.is_empty(), "{moved}: {result:?}");
    }
}

#[test]
fn checked_predicates_report_failure_responsibilities_without_consuming_success_state() {
    use elaboration::verification::moves::{CleanupReason, Delivery, analyze_in_package};
    for (condition, failures, ends) in [("true", 0, 1), ("false", 1, 0), ("flag", 1, 1)] {
        let package = compile(&format!(
            "struct Token {{}} fn run(flag:bool) {{let token=Token {{}}; assumes {condition}; let moved=token;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100000,
        )
        .unwrap();
        assert_eq!(
            report
                .cleanup_sites
                .iter()
                .filter(|s| s.reason == CleanupReason::PredicateFailure)
                .count(),
            failures,
            "{condition}: {report}"
        );
        assert_eq!(
            report
                .cleanup_sites
                .iter()
                .filter(|s| s.reason == CleanupReason::FunctionEnd)
                .count(),
            ends,
            "{condition}: {report}"
        );
        for site in report
            .cleanup_sites
            .iter()
            .filter(|s| s.reason == CleanupReason::PredicateFailure)
        {
            assert_eq!(
                site.actions
                    .iter()
                    .filter(|a| matches!(
                        a,
                        elaboration::verification::moves::CleanupAction::Owned {
                            ty: nir::Term::Nominal { .. },
                            ..
                        }
                    ))
                    .count(),
                1,
                "{report}"
            );
        }
        assert_eq!(
            report.to_string().contains("predicate-failure"),
            failures != 0
        );
    }
}

#[test]
fn checked_predicates_cannot_restore_moves_or_evaluate_unavailable_fields() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("let moved=cell; assumes flag; let again=cell;", false),
        ("let moved=cell; assumes true; let again=cell;", false),
        ("let moved=cell; assumes cell.flag;", false),
        ("let moved=cell; assumes false; let again=cell;", true),
        ("assumes cell.flag; let moved=cell;", true),
    ] {
        let package = compile(&format!(
            "struct Cell {{flag:bool,}} fn run(flag:bool) {{let cell=Cell {{flag:true}}; {body}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn checked_predicate_memory_summaries_preserve_pointer_access_obligations() {
    use elaboration::verification::{
        memory,
        moves::{Delivery, analyze_in_package},
    };
    for moved in ["", "let moved=cell;"] {
        let package = compile(&format!(
            "struct Cell {{flag:bool,}} fn checked(flag:bool,p:*bool)->bool {{assumes flag; p.*}} fn run(flag:bool) {{let cell=Cell {{flag:true}}; let p=cell.flag.ref; {moved} let seen=checked(flag,p);}}"
        ));
        let checked = package
            .functions
            .iter()
            .find(|f| f.name == "checked")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, checked.declaration, &[], &mut 100000).unwrap();
        assert_eq!(summary.accesses.len(), 1, "{summary}");
        assert_eq!(summary.accesses[0].target.parameter, 1);
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), moved.is_empty(), "{result:?}");
    }
}

#[test]
fn checked_predicate_failure_preserves_pending_constructor_responsibilities() {
    use elaboration::verification::moves::{CleanupReason, Delivery, analyze_in_package};
    let package = compile(
        "struct First {} struct Second {} fn run(flag:bool) {let value=(First {},do {assumes flag; Second {}});}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        100000,
    )
    .unwrap();
    assert_eq!(report.incomplete.len(), 1, "{report}");
    assert_eq!(report.incomplete[0].values.len(), 1, "{report}");
    let nir::Term::Nominal { declaration, .. } = &report.incomplete[0].values[0].ty else {
        panic!("prepared component")
    };
    assert_eq!(
        package
            .structs
            .iter()
            .find(|s| s.declaration == *declaration)
            .unwrap()
            .name,
        "First"
    );
    assert!(
        report
            .cleanup_sites
            .iter()
            .any(|s| s.reason == CleanupReason::PredicateFailure)
    );
    assert!(
        report
            .cleanup_sites
            .iter()
            .any(|s| s.reason == CleanupReason::FunctionEnd)
    );
}

#[test]
fn checked_predicate_resource_analysis_rejects_forged_effectful_conditions() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::ExprKind as K;
    let package = compile("fn run(flag:bool) {assumes flag;}");
    for corruption in 0..2 {
        let mut run = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .clone();
        let K::Block { statements, result } = &mut run.body.kind else {
            panic!("block")
        };
        let check = statements
            .iter_mut()
            .chain(result.as_deref_mut())
            .find(|e| matches!(e.kind, K::CheckPredicate(_)))
            .unwrap();
        let K::CheckPredicate(condition) = &mut check.kind else {
            unreachable!()
        };
        if corruption == 0 {
            check.ty = nir::Term::Primitive(nir::Primitive::Bool);
        } else {
            condition.kind = K::Return(None);
        }
        assert!(analyze_in_package(&package, &run, |_| Ok(Delivery::Copy), 100000).is_err());
        assert!(elaboration::verification::memory::infer(&run, &mut 100000).is_err());
    }
}

#[test]
fn checked_closure_failures_do_not_create_normal_continuations() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (condition, valid, successes) in
        [("false", true, 0), ("true", false, 0), ("flag", false, 0)]
    {
        let package = compile(&format!(
            "struct Token {{}} fn run(flag:bool) {{let token=Token {{}}; let moved=token; let action=|| {{assumes {condition};}}; action(); let again=token;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{condition}: {result:?}");
        if let Ok(report) = result {
            assert_eq!(report.exits.len(), successes, "{report}");
            assert_eq!(report.failed_exits.len(), 1, "{report}");
            assert!(
                report
                    .closures
                    .iter()
                    .all(|c| c.invocation.exits.is_empty()),
                "{report}"
            );
            assert!(
                report
                    .to_string()
                    .contains("(failed-exits (ownership-state")
            );
        }
    }
}

#[test]
fn checked_closure_failure_paths_propagate_through_nested_invocations() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Token {} fn run() {let token=Token {}; let moved=token; let outer=|| {let inner=|| {assumes false;}; inner();}; outer(); let again=token;}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        100000,
    )
    .unwrap();
    assert!(report.exits.is_empty(), "{report}");
    assert_eq!(report.failed_exits.len(), 1, "{report}");
}

#[test]
fn checked_closure_success_state_excludes_capture_consumption_on_failure() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Token {} fn take(value:Token) {} fn run(flag:bool,value:Token) {let action=|| {if flag {take(value); assumes false;}}; action(); action();}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        100000,
    )
    .unwrap();
    assert_eq!(report.exits.len(), 1, "{report}");
    assert_eq!(report.failed_exits.len(), 2, "{report}");
    for site in report
        .cleanup_sites
        .iter()
        .filter(|s| s.reason == elaboration::verification::moves::CleanupReason::PredicateFailure)
    {
        let mut found = false;
        for action in &site.actions {
            if let elaboration::verification::moves::CleanupAction::Owned {
                ty: nir::Term::Closure { environment, .. },
                remaining,
            } = action
            {
                let index = environment
                    .iter()
                    .position(|ty| matches!(ty, nir::Term::Nominal { .. }))
                    .unwrap();
                assert!(
                    remaining
                        .children
                        .as_ref()
                        .unwrap()
                        .iter()
                        .all(|child| child.place.fields != vec![index]),
                    "consumed capture scheduled again: {report}"
                );
                found = true;
            }
        }
        assert!(found, "missing closure responsibility: {report}");
    }
}

#[test]
fn resource_signature_contracts_cover_entry_snapshots_and_each_return_path() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (source, names) in [
        (
            include_str!("../../../development/examples/program-postconditions.fl"),
            vec![
                "identity",
                "next",
                "early",
                "branches",
                "closures",
                "total_bool",
                "parameters",
                "checked",
                "pair",
                "changed",
                "shadow",
            ],
        ),
        (
            include_str!("../../../development/examples/program-entry-snapshots.fl"),
            vec![
                "changed",
                "increment",
                "conditional",
                "bounded",
                "after_call",
            ],
        ),
    ] {
        let package = compile(source);
        for name in names {
            let function = package.functions.iter().find(|f| f.name == name).unwrap();
            analyze_in_package(
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
                100000,
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}

#[test]
fn resource_precondition_failures_stop_body_and_keep_acquired_parameters() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (condition, valid) in [("false", true), ("true", false), ("flag", false)] {
        let package = compile(&format!(
            "struct Token {{}} fn run(value:Token,flag:bool) where requires flag {{let first=value; let second=value;}}"
        ));
        let mut function = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .clone();
        if condition != "flag" {
            function.preconditions[0].kind =
                nir::program::ExprKind::Literal(nir::program::Literal::Bool(condition == "true"));
        }
        let result = analyze_in_package(
            &package,
            &function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{condition}: {result:?}");
        if let Ok(report) = result {
            assert!(report.exits.is_empty(), "{report}");
            assert_eq!(report.failed_exits.len(), 1, "{report}");
        }
    }
}

#[test]
fn resource_postcondition_failure_keeps_named_and_unnamed_owned_results() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for result in ["result:Token", "Token"] {
        for body in [
            "Token {}",
            "return Token {};",
            "if flag {return Token {};} Token {}",
        ] {
            let package = compile(&format!(
                "struct Token {{}} fn build(flag:bool)->{result} where ensures true {{{body}}}"
            ));
            let mut function = package
                .functions
                .iter()
                .find(|f| f.name == "build")
                .unwrap()
                .clone();
            function.postconditions[0].kind =
                nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
            let report = analyze_in_package(
                &package,
                &function,
                |ty| {
                    Ok(if matches!(ty, nir::Term::Primitive(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    })
                },
                100000,
            )
            .unwrap();
            assert!(report.exits.is_empty(), "{result}, {body}: {report}");
            assert_eq!(
                report.failed_exits.len(),
                if body.starts_with("if") { 2 } else { 1 },
                "{report}"
            );
            if result == "Token" {
                assert_eq!(
                    report.incomplete.len(),
                    report.failed_exits.len(),
                    "{report}"
                );
            } else {
                assert!(
                    report
                        .cleanup_sites
                        .iter()
                        .any(|s| s.actions.iter().any(|a| matches!(
                            a,
                            elaboration::verification::moves::CleanupAction::Owned {
                                ty: nir::Term::Nominal { .. },
                                ..
                            }
                        ))),
                    "{report}"
                );
            }
        }
    }
}

#[test]
fn resource_postconditions_do_not_apply_to_nested_closure_returns() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "fn run()->result:bool where ensures true {let action=|| {return true;}; let ignored=action(); true}",
    );
    let mut function = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .clone();
    function.postconditions[0].kind =
        nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let report = analyze_in_package(&package, &function, |_| Ok(Delivery::Copy), 100000).unwrap();
    assert_eq!(report.failed_exits.len(), 1, "{report}");
    assert!(report.exits.is_empty(), "{report}");
    for closure in &report.closures {
        assert!(closure.invocation.failed_exits.is_empty(), "{report}");
        assert_eq!(closure.invocation.exits.len(), 1, "{report}");
    }
}

#[test]
fn resource_entry_snapshots_reject_forged_sources_and_owned_duplication() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile("struct Token {} fn run(value:Token) {}");
    for missing in [false, true] {
        let mut function = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .clone();
        let parameter = function.parameters[0];
        function.entry_snapshots.push(nir::program::EntrySnapshot {
            parameter: if missing {
                nir::program::LocalId {
                    index: 9999,
                    ..parameter
                }
            } else {
                parameter
            },
            binding: nir::program::LocalId {
                index: 9998,
                ..parameter
            },
        });
        let error = analyze_in_package(
            &package,
            &function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100000,
        )
        .unwrap_err();
        assert!(
            error.reason.contains(if missing {
                "not a parameter"
            } else {
                "Copy permission"
            }),
            "{error}"
        );
    }
}

#[test]
fn memory_rejection_summaries_converge_through_recursive_calls() {
    use elaboration::verification::memory;
    for (condition, expected) in [("true", false), ("flag", true)] {
        let package = compile(&format!(
            "fn checked(flag:bool,p:*bool)->bool {{assumes {condition}; p.*}} fn relay(flag:bool,p:*bool)->bool {{if flag {{checked(flag,p)}} else {{relay(true,p)}}}}"
        ));
        for name in ["checked", "relay"] {
            let function = package.functions.iter().find(|f| f.name == name).unwrap();
            let summary =
                memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
            assert_eq!(summary.may_reject, expected, "{condition}: {summary}");
            assert!(
                summary
                    .to_string()
                    .contains(&format!("(may-reject {expected})"))
            );
        }
    }
}

#[test]
fn summarized_call_rejection_keeps_caller_owners_and_pending_components() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for condition in ["true", "flag"] {
        for expression in [
            "checked(flag,value.ref)",
            "(Token {},checked(flag,value.ref))",
        ] {
            let package = compile(&format!(
                "struct Token {{}} fn checked(flag:bool,p:*bool)->bool {{assumes {condition}; p.*}} fn run(flag:bool) {{let owner=Token {{}}; let value=true; let result={expression}; let moved=owner;}}"
            ));
            let function = package.functions.iter().find(|f| f.name == "run").unwrap();
            let report = analyze_in_package(
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
                100000,
            )
            .unwrap();
            assert_eq!(
                report.failed_exits.len(),
                usize::from(condition == "flag"),
                "{report}"
            );
            assert_eq!(report.exits.len(), 1, "{report}");
            assert_eq!(
                report.incomplete.len(),
                usize::from(condition == "flag" && expression.starts_with('(')),
                "{report}"
            );
        }
    }
}

#[test]
fn memory_rejection_summary_includes_signature_checks_and_keeps_access_requirements() {
    use elaboration::verification::{
        memory,
        moves::{Delivery, analyze_in_package},
    };
    let package = compile(
        "struct Cell {flag:bool,} fn checked(flag:bool,p:*bool)->result:bool where requires flag, ensures flag {p.*} fn run(flag:bool) {assumes flag; let cell=Cell {flag:true}; let p=cell.flag.ref; let moved=cell; let result=checked(flag,p);}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "checked")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(summary.may_reject, "{summary}");
    assert_eq!(summary.accesses.len(), 1, "{summary}");
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    assert!(
        analyze_in_package(
            &package,
            run,
            |ty| Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                }
            ),
            100000
        )
        .is_err()
    );
}

#[test]
fn memory_owned_argument_footprints_check_layout_and_retain_reads() {
    use elaboration::verification::memory;
    for (field, value, reads) in [("bool", "p.*", true), ("*bool", "p", false)] {
        let package = compile(&format!(
            "struct Item {{value:{field},}} fn construct(p:*bool)->Item {{Item {{value:{value}}}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "construct")
            .unwrap();
        let nir::Term::Function { parameters, .. } = function.signature.callable_signature() else {
            panic!("signature")
        };
        let prepared = [(function.parameters[0], parameters[0].ty.clone(), 3)];
        let summary =
            memory::infer_argument(&package, &function.body, &prepared, &mut 100000).unwrap();
        if reads {
            assert_eq!(summary.accesses.len(), 1, "{summary}");
            assert_eq!(summary.accesses[0].target.parameter, 3);
            assert!(summary.accesses[0].read_value);
            assert!(!summary.accesses[0].write);
            assert!(summary.result_targets.is_none());
            assert!(summary.pointer_fields.is_empty());
        } else {
            assert!(summary.accesses.is_empty());
            let targets = summary
                .pointer_fields
                .get(&vec![0])
                .unwrap()
                .as_ref()
                .unwrap();
            assert_eq!(targets.len(), 1);
            let target = targets.first().unwrap();
            assert_eq!(target.parameter, 3);
            assert!(
                target.input_fields.is_empty()
                    && target.through.is_empty()
                    && target.fields.is_empty()
            );
            assert!(
                summary
                    .to_string()
                    .contains("(pointer-fields (leaf (path 0)")
            );
        }
        assert!(memory::infer_argument(&package, &function.body, &prepared, &mut 0).is_err());
        assert_eq!(
            memory::infer_argument(&package, &function.body, &prepared, &mut 100000).unwrap(),
            summary
        );
    }
}

#[test]
fn memory_argument_rejection_preserves_prepared_pointer_slots() {
    use elaboration::verification::memory;
    let package = compile("fn checked(flag:bool,p:*bool)->bool {assumes flag; p.*}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "checked")
        .unwrap();
    let nir::Term::Function { parameters, .. } = function.signature.callable_signature() else {
        panic!("signature")
    };
    let prepared: Vec<_> = function
        .parameters
        .iter()
        .zip(parameters)
        .enumerate()
        .map(|(index, (binding, p))| (*binding, p.ty.clone(), index))
        .collect();
    let summary = memory::infer_argument(&package, &function.body, &prepared, &mut 100000).unwrap();
    assert!(summary.may_reject, "{summary}");
    assert_eq!(summary.accesses[0].target.parameter, 1);
    assert!(summary.to_string().contains("(may-reject true)"));
}

#[test]
fn ordinary_call_rejection_excludes_transferred_arguments_from_caller_cleanup() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    let package = compile(
        "struct Caller {} struct Argument {} fn consume(value:Argument,flag:bool)->bool {assumes flag; true} fn run(flag:bool) {let held=Caller {}; let passed=Argument {}; let result=consume(passed,flag); let moved=held;}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        100000,
    )
    .unwrap();
    assert_eq!(report.failed_exits.len(), 1, "{report}");
    let site = report
        .cleanup_sites
        .iter()
        .find(|s| s.reason == CleanupReason::PredicateFailure)
        .unwrap();
    let names: Vec<_> = site
        .actions
        .iter()
        .filter_map(|action| match action {
            CleanupAction::Owned {
                ty: nir::Term::Nominal { declaration, .. },
                ..
            } => Some(
                package
                    .structs
                    .iter()
                    .find(|s| s.declaration == *declaration)
                    .unwrap()
                    .name
                    .as_str(),
            ),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["Caller"], "{report}");
    assert!(
        report.incomplete.is_empty(),
        "completed arguments retained in caller: {report}"
    );
}

#[test]
fn ordinary_call_rejection_traverses_recursive_helpers_but_not_dormant_closures() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, expected) in [
        ("assumes flag; true", 1),
        ("if flag {assumes flag; true} else {helper(true)}", 1),
        ("if flag {true} else {helper(true)}", 0),
        ("let dormant=|| {assumes false;}; true", 0),
        ("false and do {assumes flag; true}", 0),
        ("true or do {assumes flag; false}", 0),
    ] {
        let package = compile(&format!(
            "struct Token {{}} fn helper(flag:bool)->bool {{{body}}} fn run(flag:bool) {{let owner=Token {{}}; let result=helper(flag);}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100000,
        )
        .unwrap();
        assert_eq!(report.failed_exits.len(), expected, "{body}: {report}");
    }
}

#[test]
fn ordinary_call_rejection_preserves_outer_preparation_after_default_arguments() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Token {} fn checked(.flag:bool=true)->bool {assumes flag; true} fn run() {let value=(Token {},checked());}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let report = analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        100000,
    )
    .unwrap();
    assert_eq!(report.failed_exits.len(), 1, "{report}");
    assert_eq!(report.incomplete.len(), 1, "{report}");
    assert_eq!(report.incomplete[0].values.len(), 1, "{report}");
    assert_eq!(report.exits.len(), 1, "{report}");
}

#[test]
fn ordinary_call_rejection_cache_keeps_repeated_helpers_within_analysis_budget() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let mut source = String::from("fn helper19() {assumes true;} ");
    for index in 0..19 {
        source.push_str(&format!("fn helper{index}() {{helper{}();}} ", index + 1));
    }
    for index in 0..80 {
        source.push_str(&format!("fn unused{index}() {{}} "));
    }
    source.push_str("fn run() {");
    for _ in 0..120 {
        source.push_str("helper0();");
    }
    source.push('}');
    let package = compile(&source);
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    assert!(analyze_in_package(&package, run, |_| Ok(Delivery::Copy), 1).is_err());
    let report = analyze_in_package(&package, run, |_| Ok(Delivery::Copy), 4000).unwrap();
    assert!(report.failed_exits.is_empty(), "{report}");
    assert_eq!(report.exits.len(), 1, "{report}");
}

fn package_with_normalized_failing_default(condition: Option<bool>) -> nir::program::Package {
    use nir::program::ExprKind as K;
    // P3 source-default admission remains separate. Use checked body IR to
    // exercise the resource-stage parameter-frame contract explicitly.
    let mut package = compile(
        "struct Caller {} struct Input {} struct Local {} fn build(input:Input,flag:bool,.result:bool=true)->bool {result} fn generate(flag:bool)->bool {let local=Local {}; assumes flag; true} fn run(flag:bool) {let keep=Caller {}; let input=Input {}; let result=build(input,flag);} fn accepted() {run(true);} fn rejected() {run(false);}",
    );
    let binding = package
        .functions
        .iter()
        .find(|f| f.name == "build")
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
    let predicate = statements
        .iter_mut()
        .find_map(|e| match &mut e.kind {
            K::CheckPredicate(predicate) => Some(predicate),
            K::Discard(inner) => {
                if let K::CheckPredicate(predicate) = &mut inner.kind {
                    Some(predicate)
                } else {
                    None
                }
            }
            _ => None,
        })
        .expect("check predicate");
    predicate.kind = if let Some(value) = condition {
        K::Literal(nir::program::Literal::Bool(value))
    } else {
        K::Local(binding)
    };
    let run = package
        .functions
        .iter_mut()
        .find(|f| f.name == "run")
        .unwrap();
    let K::Block { statements, .. } = &mut run.body.kind else {
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
fn default_failure_separates_local_owners_from_caller_and_prepared_arguments() {
    use elaboration::verification::moves::{
        CleanupAction, CleanupReason, Delivery, analyze_in_package,
    };
    for condition in [None, Some(false), Some(true)] {
        let package = package_with_normalized_failing_default(condition);
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100000,
        )
        .unwrap();
        let failed = usize::from(condition != Some(true));
        // Once defaults finish, the callee also owns parameters whose Drop
        // behavior this shape-only analysis has not certified.
        let call_failed = usize::from(condition != Some(false));
        assert_eq!(report.failed_exits.len(), failed + call_failed, "{report}");
        assert_eq!(
            report.exits.len(),
            usize::from(condition != Some(false)),
            "{report}"
        );
        assert_eq!(
            report.calls.len(),
            usize::from(condition != Some(false)),
            "{report}"
        );
        assert_eq!(report.defaults.len(), 1, "{report}");
        let default = &report.defaults[0].evaluation;
        assert_eq!(default.failed_exits.len(), failed, "{report}");
        assert!(
            default.incomplete.is_empty(),
            "prepared args duplicated into default: {report}"
        );
        assert_eq!(report.incomplete.len(), failed, "{report}");
        if failed == 1 {
            let names = |actions: &[CleanupAction]| {
                actions
                    .iter()
                    .filter_map(|a| match a {
                        CleanupAction::Owned {
                            ty: nir::Term::Nominal { declaration, .. },
                            ..
                        } => Some(
                            package
                                .structs
                                .iter()
                                .find(|s| s.declaration == *declaration)
                                .unwrap()
                                .name
                                .clone(),
                        ),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let caller = report
                .cleanup_sites
                .iter()
                .find(|s| s.reason == CleanupReason::PredicateFailure)
                .unwrap();
            assert_eq!(names(&caller.actions), vec!["Caller"]);
            let local = default
                .cleanup_sites
                .iter()
                .find(|s| s.reason == CleanupReason::PredicateFailure)
                .unwrap();
            assert_eq!(names(&local.actions), vec!["Local"]);
            assert_eq!(report.incomplete[0].values.len(), 2, "{report}");
            let value = report.incomplete[0]
                .values
                .iter()
                .find(|v| matches!(v.ty, nir::Term::Nominal { .. }))
                .unwrap();
            let nir::Term::Nominal { declaration, .. } = &value.ty else {
                panic!("argument")
            };
            assert_eq!(
                package
                    .structs
                    .iter()
                    .find(|s| s.declaration == *declaration)
                    .unwrap()
                    .name,
                "Input"
            );
        }
        assert!(report.to_string().contains("(default-evaluation (source"));
    }
}

#[test]
fn default_failure_cannot_consume_an_aliased_prepared_owner() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    use nir::program::{Expr, ExprKind as K, LocalId};
    let mut package = package_with_normalized_failing_default(Some(false));
    let build = package
        .functions
        .iter()
        .find(|f| f.name == "build")
        .unwrap();
    let binding = build.parameters[0];
    let nir::Term::Function { parameters, .. } = build.signature.callable_signature() else {
        panic!("signature")
    };
    let ty = parameters[0].ty.clone();
    let run = package
        .functions
        .iter_mut()
        .find(|f| f.name == "run")
        .unwrap();
    let K::Block { statements, .. } = &mut run.body.kind else {
        panic!("block")
    };
    let K::Let { value, .. } = &mut statements.last_mut().unwrap().kind else {
        panic!("let")
    };
    let K::Call { arguments, .. } = &mut value.kind else {
        panic!("call")
    };
    let value = &mut arguments.iter_mut().find(|a| a.is_default).unwrap().value;
    let K::Block { statements, .. } = &mut value.kind else {
        panic!("default block")
    };
    statements.insert(
        0,
        Expr {
            source: value.source,
            ty: nir::Term::Primitive(nir::Primitive::Void),
            kind: K::Let {
                binding: LocalId {
                    owner: binding.owner,
                    index: 9999,
                },
                value: Box::new(Expr {
                    source: value.source,
                    ty,
                    kind: K::Local(binding),
                }),
            },
        },
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let error = analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(if matches!(ty, nir::Term::Primitive(_)) {
                Delivery::Copy
            } else {
                Delivery::Move
            })
        },
        100000,
    )
    .unwrap_err();
    assert!(
        error
            .reason
            .contains("default failure consumed a prepared parameter"),
        "{error}"
    );
}

#[test]
fn normalized_default_failure_matches_runtime_parameter_frame_order() {
    use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
    let package = package_with_normalized_failing_default(None);
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let build = package
        .functions
        .iter()
        .find(|f| f.name == "build")
        .unwrap()
        .declaration;
    for flag in [false, true] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let result = vm.invoke(run, vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Unit);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            vm.events()
                .iter()
                .filter(|e| matches!(e,Event::EnterFunction(id) if *id==build))
                .count(),
            usize::from(flag)
        );
        assert_eq!(
            vm.events()
                .iter()
                .filter(|e| matches!(e, Event::CheckPredicate { .. }))
                .count(),
            1
        );
    }
    assert_eq!(
        luna::execution::interpret(&package, "accepted", 100000).unwrap(),
        Value::Unit
    );
    assert!(luna::execution::interpret(&package, "rejected", 100000).is_err());
}

fn callable_input_fixture(source: &str) -> (nir::program::Package, nir::program::Expr) {
    use nir::program::ExprKind as K;
    let package = compile(source);
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    let K::Block { statements, .. } = &entry.body.kind else {
        panic!("entry block")
    };
    let K::Let { value, .. } = &statements.last().unwrap().kind else {
        panic!("callback binding")
    };
    let definition = value.as_ref().clone();
    (package, definition)
}

#[test]
fn callable_input_named_body_records_exact_callback_premise() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let (package, definition) = callable_input_fixture(
        "fn apply(callback:fn()->bool)->bool {callback()} fn entry() {let callback=|| true;}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let inputs = [CallableParameter {
        choices: vec![],
        enum_captures: Default::default(),
        fields: vec![],
        captures: Default::default(),
        alternatives: vec![],
        parameter: 0,
        definition,
    }];
    let report = moves::analyze_in_package_with_inputs(
        &package,
        function,
        |_| Ok(Delivery::Copy),
        100000,
        None,
        &[],
        &inputs,
    )
    .unwrap();
    assert_eq!(report.callable_parameters, inputs);
    assert!(report.deferred_calls.is_empty());
    assert_eq!(report.closures.iter().filter(|c| c.invoked).count(), 1);
    println!("CALLABLE_INPUT_DUMP={report}\nCALLABLE_INPUT_END");
}

#[test]
fn callable_input_named_body_rejects_repeated_owned_capture_transfer() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let (package, definition) = callable_input_fixture(
        "struct Item {} fn take(item:Item) {} fn apply(callback:%{item:Item} fn()) {callback();callback();} fn entry() {let item=Item {};let callback=|| take(item);}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let error = moves::analyze_in_package_with_inputs(
        &package,
        function,
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
        None,
        &[],
        &[CallableParameter {
            choices: vec![],
            enum_captures: Default::default(),
            fields: vec![],
            captures: Default::default(),
            alternatives: vec![],
            parameter: 0,
            definition,
        }],
    )
    .unwrap_err();
    assert!(
        error.reason.contains("move") || error.reason.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn callable_input_named_body_keeps_distinct_same_typed_definitions() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let (package, definition) = callable_input_fixture(
        "fn apply(callback:fn()->bool)->bool {callback()} fn entry() {let callback=|| true;}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let mut other = definition.clone();
    let nir::program::ExprKind::Closure { body, .. } = &mut other.kind else {
        panic!("closure")
    };
    std::sync::Arc::make_mut(body).kind =
        nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let mut reports = vec![];
    for definition in [definition, other] {
        reports.push(
            moves::analyze_in_package_with_inputs(
                &package,
                function,
                |_| Ok(Delivery::Copy),
                100000,
                None,
                &[],
                &[CallableParameter {
                    choices: vec![],
                    enum_captures: Default::default(),
                    fields: vec![],
                    captures: Default::default(),
                    alternatives: vec![],
                    parameter: 0,
                    definition,
                }],
            )
            .unwrap(),
        );
    }
    assert_ne!(
        reports[0].callable_parameters,
        reports[1].callable_parameters
    );
    assert_ne!(
        reports[0].closures[0].definition,
        reports[1].closures[0].definition
    );
}

#[test]
fn callable_input_rejects_invalid_slot_type_definition_and_binding_shapes() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    use nir::program::ExprKind as K;
    let (package, definition) = callable_input_fixture(
        "fn apply(callback:fn()->bool)->bool {callback()} fn entry() {let callback=|| true;}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let input = CallableParameter {
        choices: vec![],
        enum_captures: Default::default(),
        fields: vec![],
        captures: Default::default(),
        alternatives: vec![],
        parameter: 0,
        definition,
    };
    let mut missing = input.clone();
    missing.parameter = 1;
    let mut wrong_type = input.clone();
    wrong_type.definition.ty = nir::Term::Primitive(nir::Primitive::Bool);
    let mut not_closure = input.clone();
    not_closure.definition.kind = K::Unit;
    let mut wrong_arity = input.clone();
    let K::Closure { parameters, .. } = &mut wrong_arity.definition.kind else {
        panic!("closure")
    };
    parameters.push(function.parameters[0]);
    for (inputs, message) in [
        (
            vec![input.clone(), input.clone()],
            "duplicate callable parameter",
        ),
        (vec![missing], "callable parameter is missing"),
        (vec![wrong_type], "type disagrees"),
        (vec![not_closure], "closure definition"),
        (vec![wrong_arity], "shape disagrees"),
    ] {
        let error = moves::analyze_in_package_with_inputs(
            &package,
            function,
            |_| Ok(Delivery::Copy),
            100000,
            None,
            &[],
            &inputs,
        )
        .unwrap_err();
        assert!(error.reason.contains(message), "{error}");
    }
}

#[test]
fn callable_input_named_definition_validates_identity_phase_signature_and_arity() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    use nir::program::ExprKind as K;
    let (package, template) = callable_input_fixture(
        "struct NotFunction {} fn apply(callback:fn()->bool)->bool {callback()} fn valid()->bool {true} fn mismatch()->'x''type {'x'} pure comptime fn staged()->bool {true} fn generic(value:T)->T where T {value} fn entry() {let callback=|| true;}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    for (name, message) in [
        ("valid", None),
        ("mismatch", Some("signature mismatch")),
        ("staged", Some("runtime function")),
        ("generic", Some("type arguments")),
    ] {
        let mut definition = template.clone();
        definition.kind = K::Function(
            package
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .declaration,
        );
        let result = moves::analyze_in_package_with_inputs(
            &package,
            function,
            |_| Ok(Delivery::Copy),
            100000,
            None,
            &[],
            &[CallableParameter {
                choices: vec![],
                enum_captures: Default::default(),
                fields: vec![],
                captures: Default::default(),
                alternatives: vec![],
                parameter: 0,
                definition,
            }],
        );
        if let Some(message) = message {
            let error = result.unwrap_err();
            assert!(error.reason.contains(message), "{error}");
        } else {
            assert!(result.is_ok(), "{result:?}");
        }
    }
    let mut definition = template;
    definition.kind = K::Function(package.structs[0].declaration);
    let error = moves::analyze_in_package_with_inputs(
        &package,
        function,
        |_| Ok(Delivery::Copy),
        100000,
        None,
        &[],
        &[CallableParameter {
            choices: vec![],
            enum_captures: Default::default(),
            fields: vec![],
            captures: Default::default(),
            alternatives: vec![],
            parameter: 0,
            definition,
        }],
    )
    .unwrap_err();
    assert!(error.reason.contains("unknown function"), "{error}");
}

#[test]
fn named_value_memory_call_rejects_malformed_parameter_frame_without_panicking() {
    use elaboration::verification::moves::{self, Delivery};
    use nir::program::ExprKind as K;
    for duplicate in [false, true] {
        let mut package = compile(
            "fn change(left:*bool,right:*bool) {left.* = false;right.* = true;} fn entry() {let left=true;let right=false;change(left.ref,right.ref);}",
        );
        let entry = package
            .functions
            .iter_mut()
            .find(|f| f.name == "entry")
            .unwrap();
        let K::Block { statements, .. } = &mut entry.body.kind else {
            panic!("block")
        };
        let statement = statements.last_mut().unwrap();
        let call = if let K::Discard(value) = &mut statement.kind {
            value.as_mut()
        } else {
            statement
        };
        let K::Call {
            parameter_bindings, ..
        } = &mut call.kind
        else {
            panic!("call")
        };
        if duplicate {
            parameter_bindings[1] = parameter_bindings[0];
        } else {
            parameter_bindings.clear();
        }
        let entry = package
            .functions
            .iter()
            .find(|f| f.name == "entry")
            .unwrap();
        let error =
            moves::analyze_in_package(&package, entry, |_| Ok(Delivery::Copy), 100000).unwrap_err();
        assert!(
            error
                .reason
                .contains("complete unique parameter binding frame"),
            "{error}"
        );
    }
}

#[test]
fn callable_result_unknown_call_cannot_inherit_an_unrelated_block_local() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "typealias Callback = fn()->bool; fn apply(callback:Callback)->bool {callback()} fn entry(factory:fn()->Callback)->bool {apply(do {let unrelated=|| false;factory()})}",
    );
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    let report =
        moves::analyze_in_package(&package, entry, |_| Ok(Delivery::Copy), 100000).unwrap();
    assert_eq!(report.deferred_calls.len(), 1, "{report}");
    assert!(report.named_calls.is_empty(), "{report}");
    assert_eq!(report.callable_exits, [None]);
}

#[test]
fn callable_result_report_records_only_successful_exit_definitions() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "typealias Callback = fn()->bool; fn selected()->bool {true} fn factory(flag:bool)->Callback {if flag {assumes false;}selected}",
    );
    let factory = package
        .functions
        .iter()
        .find(|f| f.name == "factory")
        .unwrap();
    let selected = package
        .functions
        .iter()
        .find(|f| f.name == "selected")
        .unwrap()
        .declaration;
    let report =
        moves::analyze_in_package(&package, factory, |_| Ok(Delivery::Copy), 100000).unwrap();
    assert_eq!(report.failed_exits.len(), 1);
    assert_eq!(report.exits.len(), 1);
    assert_eq!(report.callable_exits.len(), 1);
    assert!(
        matches!(report.callable_exits[0].as_ref().unwrap().kind, nir::program::ExprKind::Function(id) if id == selected)
    );
    println!("CALLABLE_EXIT_DUMP={report}\nCALLABLE_EXIT_END");
}

#[test]
fn named_return_recursive_dependency_stays_unknown_and_bounded() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "typealias Callback=fn()->bool; fn recursive()->Callback {recursive()} fn entry()->Callback {recursive()}",
    );
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    let report = moves::analyze_in_package(&package, entry, |_| Ok(Delivery::Copy), 1000).unwrap();
    assert_eq!(report.callable_exits, [None]);
    assert!(report.entry_steps_used > 0 && report.entry_steps_used <= 1000);
}

#[test]
fn named_return_nested_analysis_charges_budget_and_recovers() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "typealias Callback=fn()->bool; fn selected()->bool {true} fn third()->Callback {selected} fn second()->Callback {third()} fn first()->Callback {second()} fn entry()->Callback {first()}",
    );
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    assert!(moves::analyze_in_package(&package, entry, |_| Ok(Delivery::Copy), 10).is_err());
    let report = moves::analyze_in_package(&package, entry, |_| Ok(Delivery::Copy), 10000).unwrap();
    assert!(report.entry_steps_used > 10 && report.entry_steps_used <= 10000);
    assert!(matches!(
        report.callable_exits[0].as_ref().unwrap().kind,
        nir::program::ExprKind::Function(_)
    ));
    println!("NAMED_RETURN_REPORT_DUMP={report}\nNAMED_RETURN_REPORT_END");
}

#[test]
fn memory_factory_code_value_does_not_execute_the_returned_body() {
    use elaboration::verification::memory;
    let package = compile(
        "typealias Setter=fn(*bool)->*bool; fn set(value:*bool)->*bool {value.* = true;value} fn make()->Setter {set}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let set = package.functions.iter().find(|f| f.name == "set").unwrap();
    let factory = memory::infer_in_package(&package, make.declaration, &[], &mut 10000).unwrap();
    assert!(factory.accesses.is_empty(), "{factory}");
    assert!(
        factory.result_targets.is_none(),
        "code is not a returned data pointer: {factory}"
    );
    let invocation = memory::infer_in_package(&package, set.declaration, &[], &mut 10000).unwrap();
    assert!(!invocation.accesses.is_empty(), "{invocation}");
    assert!(
        invocation
            .result_targets
            .as_ref()
            .is_some_and(|targets| !targets.is_empty())
    );
}

#[test]
fn memory_local_named_aliases_keep_exact_results_and_argument_order() {
    use elaboration::verification::memory;
    for body in [
        "let helper=first;helper(a,b)",
        "let helper=first;let next=helper;next(a,b)",
        "var helper=first;if flag {helper=first;} else {helper=first;} helper(a,b)",
        "var helper=first;if false {helper=second;} helper(a,b)",
        "var helper=first;helper(a,do {helper=second;b})",
    ] {
        let package = compile(&format!(
            "fn first(a:*bool,b:*bool)->*bool {{a}} fn second(a:*bool,b:*bool)->*bool {{b}} fn wrapper(a:*bool,b:*bool,flag:bool)->*bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let targets = summary.result_targets.as_ref().unwrap();
        assert_eq!(targets.len(), 1, "{body}: {summary}");
        assert_eq!(
            targets.iter().next().unwrap().parameter,
            0,
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_local_named_aliases_union_live_paths_and_loop_heads() {
    use elaboration::verification::memory;
    for body in [
        "var helper=first;if flag {helper=second;} helper(a,b)",
        "var helper=first;flag match {true=>{helper=second;},false=>{},};helper(a,b)",
        "var helper=first;flag and do {helper=second;true};helper(a,b)",
        "var helper=first;var again=flag;while again {let pointer=helper(a,b);helper=second;again=false;} helper(a,b)",
    ] {
        let package = compile(&format!(
            "fn first(a:*bool,b:*bool)->*bool {{a}} fn second(a:*bool,b:*bool)->*bool {{b}} fn wrapper(a:*bool,b:*bool,flag:bool)->*bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let expected = if body.ends_with("}a") {
            vec![0]
        } else {
            vec![0, 1]
        };
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .map(|p| p.parameter)
                .collect::<Vec<_>>(),
            expected,
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_local_named_aliases_instantiate_generic_targets_and_recover_after_budget_failure() {
    use elaboration::verification::memory;
    let package = compile(
        "fn alias(value:*T)->*T where T {value} fn wrapper(value:*T)->*T where T {let helper:fn(*T)->*T=alias;helper(value)}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    for ty in [nir::Primitive::Bool, nir::Primitive::Char] {
        let types = [nir::Term::Primitive(ty)];
        assert!(memory::infer_in_package(&package, function.declaration, &types, &mut 1).is_err());
        let summary =
            memory::infer_in_package(&package, function.declaration, &types, &mut 100000).unwrap();
        assert_eq!(summary.arguments, types);
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0
        );
        println!("MEMORY_ALIAS_DUMP={summary}\nMEMORY_ALIAS_END");
    }
}

#[test]
fn memory_callable_expression_results_preserve_branch_time_identity() {
    use elaboration::verification::memory;
    for body in [
        "let helper=do {let inner=first;inner};helper(a,b)",
        "let helper=if flag {first} else {first};helper(a,b)",
        "let helper=flag match {true=>first,false=>first,};helper(a,b)",
        "var helper=first;let selected=if flag {helper} else {helper=second;first};selected(a,b)",
        "var helper=first;let selected=flag match {true=>helper,false=>{helper=second;first},};selected(a,b)",
        "(if flag {first} else {first})(a,b)",
        "(do {let helper=first;helper})(a,b)",
        "let helper=if flag {return b;} else {first};helper(a,b)",
    ] {
        let package = compile(&format!(
            "fn first(a:*bool,b:*bool)->*bool {{a}} fn second(a:*bool,b:*bool)->*bool {{b}} fn wrapper(a:*bool,b:*bool,flag:bool)->*bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let targets = summary.result_targets.as_ref().unwrap();
        let expected = if body.contains("return b") {
            vec![0, 1]
        } else {
            vec![0]
        };
        assert_eq!(
            targets.iter().map(|p| p.parameter).collect::<Vec<_>>(),
            expected,
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_callable_expression_results_union_disagreeing_targets() {
    use elaboration::verification::memory;
    for body in [
        "let helper=if flag {first} else {second};helper(a,b)",
        "let helper=flag match {true=>first,false=>second,};helper(a,b)",
        "var helper=first;let selected=if flag {helper} else {helper=second;helper};selected(a,b)",
        "var helper=first;while flag {let selected=if flag {helper} else {first};let out=selected(a,b);helper=second;}a",
    ] {
        let package = compile(&format!(
            "fn first(a:*bool,b:*bool)->*bool {{a}} fn second(a:*bool,b:*bool)->*bool {{b}} fn wrapper(a:*bool,b:*bool,flag:bool)->*bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let expected = if body.ends_with("}a") {
            vec![0]
        } else {
            vec![0, 1]
        };
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .map(|p| p.parameter)
                .collect::<Vec<_>>(),
            expected,
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_callable_expression_results_keep_instantiated_type_arguments() {
    use elaboration::verification::memory;
    let package = compile(
        "fn alias(value:*T)->*T where T {value} fn wrapper(value:*T,flag:bool)->*T where T {let helper:fn(*T)->*T=if flag {alias} else {alias};helper(value)}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    for ty in [nir::Primitive::Bool, nir::Primitive::Char] {
        let types = [nir::Term::Primitive(ty)];
        let summary =
            memory::infer_in_package(&package, function.declaration, &types, &mut 100000).unwrap();
        assert_eq!(summary.arguments, types);
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0
        );
    }
}

#[test]
fn memory_callable_factory_summaries_cross_named_calls_and_keep_result_targets() {
    use elaboration::verification::memory::{self, CallableResult};
    for factory in [
        "fn make()->Alias {alias}",
        "fn make()->Alias {return alias;}",
        "fn inner()->Alias {alias} fn make()->Alias {inner()}",
        "fn make()->Alias {if true {alias} else {alias}}",
    ] {
        let package = compile(&format!(
            "typealias Alias=fn(*bool)->*bool; fn alias(value:*bool)->*bool {{value}} {factory} fn wrapper(value:*bool)->*bool {{let helper=make();helper(value)}}"
        ));
        let make = package.functions.iter().find(|f| f.name == "make").unwrap();
        let alias = package
            .functions
            .iter()
            .find(|f| f.name == "alias")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, make.declaration, &[], &mut 100000).unwrap();
        assert!(
            matches!(summary.callable_result,CallableResult::Known(ref key) if key.declaration==alias.declaration),
            "{summary}"
        );
        println!("FACTORY_SUMMARY={summary}\nFACTORY_SUMMARY_END");
        let wrapper = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0,
            "{summary}"
        );
    }
}

#[test]
fn memory_callable_factory_fixed_point_distinguishes_bottom_and_candidates() {
    use elaboration::verification::memory::{self, CallableResult};
    let package = compile(
        "typealias Alias=fn(*bool)->*bool; fn first(value:*bool)->*bool {value} fn second(value:*bool)->*bool {value} fn recur(flag:bool)->Alias {if flag {first} else {recur(flag)}} fn forever()->Alias {forever()} fn choose(flag:bool)->Alias {if flag {first} else {second}} fn wrapper(value:*bool,flag:bool)->*bool {choose(flag)(value)}",
    );
    for (name, expected) in [
        ("recur", "known"),
        ("forever", "bottom"),
        ("choose", "candidates"),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err());
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert!(
            matches!(
                (&summary.callable_result, expected),
                (CallableResult::Known(_), "known")
                    | (CallableResult::NoReturn, "bottom")
                    | (CallableResult::Candidates(_), "candidates")
            ),
            "{name}: {summary}"
        );
    }
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
    assert_eq!(summary.result_targets.as_ref().unwrap().len(), 1);
}

#[test]
fn memory_callable_factory_results_preserve_concrete_generic_instances() {
    use elaboration::verification::memory::{self, CallableResult};
    let package = compile(
        "fn alias(value:*T)->*T where T {value} fn make(value:*T)->(fn(*T)->*T) where T {alias} fn wrapper(value:*T)->*T where T {make(value)(value)}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    for ty in [nir::Primitive::Bool, nir::Primitive::Char] {
        let types = [nir::Term::Primitive(ty)];
        let summary =
            memory::infer_in_package(&package, make.declaration, &types, &mut 100000).unwrap();
        assert!(
            matches!(&summary.callable_result,CallableResult::Known(key) if key.arguments==types),
            "{summary}"
        );
        let summary =
            memory::infer_in_package(&package, wrapper.declaration, &types, &mut 100000).unwrap();
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0
        );
    }
}

#[test]
fn memory_callable_factory_worklist_revisits_arguments_after_callee_resolution() {
    use elaboration::verification::memory;
    let package = compile(
        "typealias Alias=fn(*bool)->*bool; fn alias(value:*bool)->*bool {value} fn make()->Alias {alias} fn forever()->Alias {forever()} fn live(value:*bool)->*bool {make()(do {value.* = true;value})} fn unreachable(value:*bool)->*bool {forever()(do {value.* = true;value})}",
    );
    for (name, reachable) in [("live", true), ("unreachable", false)] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary.accesses.iter().any(|access| access.write),
            reachable,
            "{summary}"
        );
        assert_eq!(
            !summary.result_targets.as_ref().unwrap().is_empty(),
            reachable,
            "{summary}"
        );
    }
}

#[test]
fn memory_candidate_calls_union_writes_rejections_and_result_targets() {
    use elaboration::verification::memory::{self, CallableResult};
    let package = compile(
        "typealias Choose=fn(*bool,*bool,bool)->*bool; fn first(a:*bool,b:*bool,flag:bool)->*bool {a.* = true;a} fn second(a:*bool,b:*bool,flag:bool)->*bool {assumes flag;b.* = false;b} fn choose(flag:bool)->Choose {if flag {first} else {second}} fn wrapper(a:*bool,b:*bool,flag:bool)->*bool {choose(flag)(a,b,flag)}",
    );
    let choose = package
        .functions
        .iter()
        .find(|f| f.name == "choose")
        .unwrap();
    let factory = memory::infer_in_package(&package, choose.declaration, &[], &mut 100000).unwrap();
    assert!(matches!(&factory.callable_result,CallableResult::Candidates(keys) if keys.len()==2));
    println!("CANDIDATE_SUMMARY={factory}\nCANDIDATE_SUMMARY_END");
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).unwrap();
    assert!(summary.may_reject, "{summary}");
    for parameter in [0, 1] {
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.write && a.target.parameter == parameter),
            "{summary}"
        );
    }
    assert_eq!(
        summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|p| p.parameter)
            .collect::<Vec<_>>(),
        [0, 1]
    );
}

#[test]
fn memory_candidate_calls_require_every_target_to_have_a_contract() {
    use elaboration::verification::memory;
    let package = compile(
        "fn good(a:*bool,b:*bool)->*bool {a} fn bad(a:*bool,b:*bool)->*bool {let local=true;local.ref} fn wrapper(a:*bool,b:*bool,flag:bool)->*bool {let helper=if flag {good} else {bad};helper(a,b)}",
    );
    let wrapper = package
        .functions
        .iter()
        .find(|f| f.name == "wrapper")
        .unwrap();
    assert!(memory::infer_in_package(&package, wrapper.declaration, &[], &mut 100000).is_err());
}

#[test]
fn memory_candidate_sets_are_canonical_and_preserve_generic_instances() {
    use elaboration::verification::memory::{self, CallableResult};
    let package = compile(
        "fn first(value:*T)->*T where T {value} fn second(value:*T)->*T where T {value} fn choose(value:*T,flag:bool)->(fn(*T)->*T) where T {if flag {first} else {second}} fn reverse(value:*T,flag:bool)->(fn(*T)->*T) where T {if flag {second} else {first}} fn wrapper(value:*T,flag:bool)->*T where T {choose(value,flag)(value)}",
    );
    for ty in [nir::Primitive::Bool, nir::Primitive::Char] {
        let types = [nir::Term::Primitive(ty)];
        let mut identities = vec![];
        for name in ["choose", "reverse"] {
            let function = package.functions.iter().find(|f| f.name == name).unwrap();
            let mut fuel = 100000;
            let summary =
                memory::infer_in_package(&package, function.declaration, &types, &mut fuel)
                    .unwrap();
            let used = 100000 - fuel;
            assert!(
                memory::infer_in_package(&package, function.declaration, &types, &mut (used - 1))
                    .is_err()
            );
            let again =
                memory::infer_in_package(&package, function.declaration, &types, &mut 100000)
                    .unwrap();
            assert_eq!(summary, again);
            assert!(
                matches!(&summary.callable_result,CallableResult::Candidates(keys) if keys.len()==2 && keys.iter().all(|key|key.arguments==types)),
                "{summary}"
            );
            identities.push(summary.callable_result);
        }
        assert_eq!(identities[0], identities[1]);
        assert_eq!(identities[0].to_string(), identities[1].to_string());
        let wrapper = package
            .functions
            .iter()
            .find(|f| f.name == "wrapper")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, wrapper.declaration, &types, &mut 100000).unwrap();
        assert_eq!(summary.result_targets.as_ref().unwrap().len(), 1);
    }
}

#[test]
fn resource_named_candidates_report_all_checked_targets_at_one_call() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);} fn first()->bool {true} fn second()->bool {false} fn entry(flag:bool)->bool {let helper=if flag {first} else {second};helper()}",
    );
    let analysis = luna::resources::analyze_with_protocols(
        &package,
        "entry",
        "Duplicate",
        Some("Retire"),
        100000,
    )
    .unwrap();
    assert_eq!(analysis.report.named_candidate_calls.len(), 1);
    let candidates = &analysis.report.named_candidate_calls[0].candidates;
    assert_eq!(candidates.len(), 2);
    assert_ne!(candidates[0].kind, candidates[1].kind);
    println!("RESOURCE_CANDIDATES={analysis}\nRESOURCE_CANDIDATES_END");
}

#[test]
fn resource_named_candidates_return_report_does_not_collapse_to_one_definition() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);} typealias Callback=fn()->bool; fn first()->bool {true} fn second()->bool {false} fn make(flag:bool)->Callback {if flag {first} else {second}}",
    );
    let analysis = luna::resources::analyze_with_protocols(
        &package,
        "make",
        "Duplicate",
        Some("Retire"),
        100000,
    )
    .unwrap();
    assert_eq!(analysis.report.callable_candidate_exits.len(), 1);
    assert_eq!(
        analysis.report.callable_candidate_exits[0]
            .as_ref()
            .unwrap()
            .len(),
        2
    );
    assert!(analysis.report.callable_exits[0].is_none());
    println!("RESOURCE_RETURN_CANDIDATES={analysis}\nRESOURCE_RETURN_CANDIDATES_END");
}

#[test]
fn callable_input_candidates_validate_every_target_and_preserve_premises() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    use nir::program::ExprKind as K;
    let (package, template) = callable_input_fixture(
        "fn apply(callback:fn()->bool)->bool {callback()} fn first()->bool {true} fn second()->bool {false} fn mismatch()->'x''type {'x'} pure comptime fn staged()->bool {true} fn generic(value:T)->T where T {value} fn entry() {let callback=|| true;}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let named = |name: &str| {
        let mut definition = template.clone();
        definition.kind = K::Function(
            package
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .declaration,
        );
        definition
    };
    let analyze = |inputs: &[CallableParameter], fuel| {
        moves::analyze_in_package_with_inputs(
            &package,
            function,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            inputs,
        )
    };
    let valid = CallableParameter {
        choices: vec![],
        enum_captures: Default::default(),
        fields: vec![],
        captures: Default::default(),
        parameter: 0,
        definition: named("first"),
        alternatives: vec![named("second")],
    };
    let report = analyze(&[valid.clone()], 100000).unwrap();
    assert_eq!(report.callable_parameters, [valid.clone()]);
    assert!(report.deferred_calls.is_empty());
    assert_eq!(report.named_candidate_calls.len(), 1);
    assert_eq!(report.named_candidate_calls[0].candidates.len(), 2);
    println!("HIGHER_CANDIDATES={report}\nHIGHER_CANDIDATES_END");
    for (alternative, message) in [
        (named("first"), "duplicate callable parameter candidate"),
        (named("mismatch"), "signature mismatch"),
        (named("staged"), "runtime function"),
        (named("generic"), "type arguments"),
        (template.clone(), "named function definitions"),
    ] {
        let mut input = valid.clone();
        input.alternatives = vec![alternative];
        let error = analyze(&[input], 100000).unwrap_err();
        assert!(error.reason.contains(message), "{error}");
    }
    let mut wrong_type = valid.clone();
    wrong_type.alternatives[0].ty = nir::Term::Primitive(nir::Primitive::Bool);
    assert!(
        analyze(&[wrong_type], 100000)
            .unwrap_err()
            .reason
            .contains("type disagrees")
    );
    assert!(analyze(&[valid.clone()], report.entry_steps_used - 1).is_err());
    assert!(analyze(&[valid], 100000).is_ok());
}

#[test]
fn captured_callable_evidence_roundtrips_and_rejects_invalid_nested_premises() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let package = compile(
        "typealias Code=fn()->bool; typealias Wrapper=%{helper:Code} fn()->bool; fn first()->bool {true} fn second()->bool {false} fn make(flag:bool)->Wrapper {let helper=if flag {first} else {second};|| helper()} fn apply(wrapper:Wrapper)->bool {wrapper()}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let apply = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let returned =
        moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    assert!(returned.callable_exits[0].is_none());
    assert!(returned.callable_candidate_exits[0].is_none());
    let evidence = returned.callable_evidence_exits[0].as_ref().unwrap();
    assert_eq!(evidence.captures.len(), 1);
    let nir::program::ExprKind::Closure { captures, .. } = &evidence.definition.kind else {
        panic!("returned closure definition");
    };
    assert!(
        captures[0].value.is_none(),
        "provenance must not rewrite a lexical capture into executable code"
    );
    let binding = evidence.captures.keys().next().unwrap().clone();
    assert_eq!(evidence.captures[&binding].alternatives.len(), 1);
    let input = CallableParameter {
        choices: vec![],
        enum_captures: Default::default(),
        fields: vec![],
        parameter: 0,
        definition: evidence.definition.clone(),
        alternatives: evidence.alternatives.clone(),
        captures: evidence.captures.clone(),
    };
    let analyze = |input: &CallableParameter, fuel| {
        moves::analyze_in_package_with_inputs(
            &package,
            apply,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            std::slice::from_ref(input),
        )
    };
    let report = analyze(&input, 100000).unwrap();
    assert_eq!(report.callable_parameters, [input.clone()]);
    let invocation = &report
        .closures
        .iter()
        .find(|closure| closure.invoked)
        .unwrap()
        .invocation;
    assert!(invocation.deferred_calls.is_empty());
    assert_eq!(invocation.named_candidate_calls[0].candidates.len(), 2);
    println!("CAPTURED_EVIDENCE={report}\nCAPTURED_EVIDENCE_END");
    let mut unknown = input.clone();
    let mut missing = binding.clone();
    missing.root.local += 1000;
    unknown
        .captures
        .insert(missing, unknown.captures[&binding].clone());
    let mut mismatch = input.clone();
    mismatch.captures.get_mut(&binding).unwrap().definition.ty =
        nir::Term::Primitive(nir::Primitive::Bool);
    let mut duplicate = input.clone();
    let nested = duplicate.captures.get_mut(&binding).unwrap();
    nested.alternatives.push(nested.definition.clone());
    let mut impossible = input.clone();
    impossible
        .captures
        .get_mut(&binding)
        .unwrap()
        .captures
        .insert(binding.clone(), evidence.captures[&binding].clone());
    for (invalid, expected) in [
        (unknown, "unknown capture"),
        (mismatch, "type disagrees"),
        (duplicate, "duplicate callable parameter candidate"),
        (impossible, "named callable cannot carry capture evidence"),
    ] {
        let error = analyze(&invalid, 100000).unwrap_err();
        assert!(error.reason.contains(expected), "{error}");
    }
    assert!(analyze(&input, report.entry_steps_used - 1).is_err());
    assert!(analyze(&input, 100000).is_ok());
}

#[test]
fn captured_callable_writeback_reports_each_normal_exit_and_joins_targets() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "fn first()->bool {true} fn second()->bool {false} fn third()->bool {true} fn entry(flag:bool)->bool {var inner=first;let outer=|flag:bool| {let previous=inner();if flag {inner=second;return previous;}inner=third;previous};outer(flag);outer(false)}",
    );
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "entry")
        .unwrap();
    let analyze = |fuel| moves::analyze_in_package(&package, entry, |_| Ok(Delivery::Copy), fuel);
    let report = analyze(100000).unwrap();
    let calls: Vec<_> = report
        .closures
        .iter()
        .filter(|closure| closure.invoked)
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].invocation.captured_callable_exits.len(), 2);
    for captures in &calls[0].invocation.captured_callable_exits {
        assert_eq!(captures.len(), 1);
        assert!(captures.values().next().unwrap().alternatives.is_empty());
    }
    assert_ne!(
        calls[0].invocation.captured_callable_exits[0],
        calls[0].invocation.captured_callable_exits[1]
    );
    assert_eq!(calls[1].invocation.named_candidate_calls.len(), 1);
    assert_eq!(
        calls[1].invocation.named_candidate_calls[0]
            .candidates
            .len(),
        2
    );
    println!("CAPTURE_WRITEBACK={report}\nCAPTURE_WRITEBACK_END");
    assert!(analyze(report.entry_steps_used - 1).is_err());
    assert!(analyze(100000).is_ok());
}

#[test]
fn variadic_collections_transfer_each_affine_input_once_in_source_order() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("take(first,second);", true),
        ("take(.items=(first,second));", true),
        ("take(first,first);", false),
        ("take(first,.flag=do {consume(first);true},second);", false),
        ("take(first,second);consume(first);", false),
        ("take(first,do {return;second});", true),
    ] {
        let package = tuple_support::compile_with_tuple(&format!(
            "struct Ticket {{}} fn consume(value:Ticket) {{}} fn take(...items:Tuple<Ticket,Ticket>,.flag:bool=false) {{}} fn run(first:Ticket,second:Ticket) {{{body}}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn tuple_spreads_move_owned_packets_without_copying_responsibility() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("let packet=(first,second);take(...packet);", true),
        (
            "let packet=(first,second);take(...packet);take(...packet);",
            false,
        ),
        (
            "let packet=(first,second);take(...packet);consume(first);",
            false,
        ),
    ] {
        let package = compile(&format!(
            "struct Ticket {{}} fn consume(value:Ticket) {{}} fn take(a:Ticket,b:Ticket) {{}} fn run(first:Ticket,second:Ticket) {{{body}}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn inherent_methods_do_not_become_pending_copy_or_drop_candidates() {
    let package = compile(
        "trait Retire {fn drop(*self);} trait Duplicate {} struct Owned {} impl Retire for Owned {fn drop(*self) {}} impl Owned {fn consume(self) {}} fn entry() {let value=Owned {};value.consume();}",
    );
    let report = luna::resources::analyze_with_protocols(
        &package,
        "entry",
        "Duplicate",
        Some("Retire"),
        100_000,
    )
    .unwrap();
    assert!(report.drop_protocol.is_some());
    let package = compile(
        "trait Duplicate {} struct Owned {} impl Owned {fn consume(self) {}} fn entry() {let value=Owned {};value.consume();value.consume();}",
    );
    let error = luna::resources::analyze(&package, "entry", "Duplicate", 100_000)
        .err()
        .expect("inherent methods cannot grant Copy");
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn property_collection_moves_fields_once_and_preserves_unmoved_siblings() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("consume(value.first);consume(value.second);", true),
        ("consume(value.first);consume(value.first);", false),
        ("consume(value.first);let whole=value;", false),
    ] {
        let package = compile(&format!(
            "struct Ticket {{}} fn keep(...properties:T)->T where T {{properties}} fn consume(value:Ticket) {{}} fn run() {{let value=keep {{first:Ticket {{}},second:Ticket {{}}}};{body}}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn anonymous_record_places_reinitialize_moves_and_reject_dead_field_access() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        (
            "consume(value.first);value.first=Ticket {flag:false};consume(value.first);consume(value.second);",
            true,
        ),
        (
            "let p=value.first.flag.ref;let consumed=value.second;p.*;",
            true,
        ),
        (
            "let p=value.first.flag.ref;let consumed=value.first;p.*;",
            false,
        ),
        (
            "let p=value.ref;let consumed=value.first;p.first.flag.*;",
            false,
        ),
        (
            "let p=value.first.flag.ref;let consumed=value.first;p.* = false;",
            false,
        ),
    ] {
        let package = compile(&format!(
            "struct Ticket {{flag:bool}} fn keep(...properties:T)->T where T {{properties}} fn consume(value:Ticket) {{}} fn run() {{let value=keep {{first:Ticket {{flag:true}},second:Ticket {{flag:false}}}};{body}}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
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
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn distinct_collection_entries_do_not_duplicate_affine_inputs() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (inputs, valid) in [
        ("first,field:Ticket {},second", true),
        ("first,field:first,second", false),
        ("field:first,first,second", false),
    ] {
        let package = tuple_support::compile_with_tuple(&format!(
            "struct Ticket {{}} fn take(...items:Tuple<Ticket,Ticket>,...properties:T) where T {{}} fn run(first:Ticket,second:Ticket) {{take {{{inputs}}};}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(result.is_ok(), valid, "{inputs}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn generic_trait_method_values_preserve_affine_argument_delivery() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("action(value);", true),
        ("action(value);action(value);", false),
    ] {
        let package = compile(&format!(
            "trait Consume {{fn consume(value:T) where T;}} struct Holder {{}} struct Ticket {{}} impl Consume for Holder {{fn consume(value:U) where U {{}}}} fn run(value:Ticket) {{let action:fn(Ticket)=Holder.as(Consume).consume;{body}}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Function { .. }) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100_000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
        if let Err(error) = result {
            assert!(error.reason.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn captured_field_evidence_checks_layout_type_and_budget() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let package = tuple_support::compile_with_tuple(
        "struct Operations {pair:Tuple<fn(bool)->bool,bool>} typealias Callback=%{ops:Operations} fn(bool)->bool; fn negate(value:bool)->bool {not value} fn make()->Callback {let ops=Operations {pair:(negate,false)};|value:bool| {let (action,_)=ops.pair;action(value)}} fn invoke(callback:Callback,value:bool)->bool {callback(value)}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let invoke = package
        .functions
        .iter()
        .find(|f| f.name == "invoke")
        .unwrap();
    let report = moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let evidence = report.callable_evidence_exits[0].as_ref().unwrap();
    let place = evidence.captures.keys().next().unwrap().clone();
    assert_eq!(place.fields, vec![0, 0]);
    assert!(evidence.to_string().contains("(fields 0 0)"));
    let input = CallableParameter {
        choices: evidence.choices.clone(),
        enum_captures: evidence.enum_captures.clone(),
        fields: vec![],
        parameter: 0,
        definition: evidence.definition.clone(),
        alternatives: evidence.alternatives.clone(),
        captures: evidence.captures.clone(),
    };
    let analyze = |input: &CallableParameter, fuel| {
        moves::analyze_in_package_with_inputs(
            &package,
            invoke,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            std::slice::from_ref(input),
        )
    };
    let valid = analyze(&input, 100000).unwrap();
    assert!(valid.deferred_calls.is_empty());
    let mut bad_path = input.clone();
    let value = bad_path.captures.remove(&place).unwrap();
    let mut out_of_bounds = place.clone();
    out_of_bounds.fields[1] = 99;
    bad_path.captures.insert(out_of_bounds, value);
    assert!(
        analyze(&bad_path, 100000)
            .unwrap_err()
            .reason
            .contains("out of bounds")
    );
    let mut bad_type = input.clone();
    bad_type.captures.get_mut(&place).unwrap().definition.ty =
        nir::Term::Primitive(nir::Primitive::Bool);
    assert!(
        analyze(&bad_type, 100000)
            .unwrap_err()
            .reason
            .contains("type disagrees")
    );
    assert!(analyze(&input, 0).is_err());
    assert!(analyze(&input, 100000).is_ok());
}

#[test]
fn aggregate_result_evidence_keeps_independent_exit_paths() {
    use elaboration::verification::moves::{self, Delivery};
    let package = tuple_support::compile_with_tuple(
        "fn known()->bool {true} fn make(flag:bool,action:fn()->bool)->Tuple<fn()->bool,bool> {if flag {return (known,true);}(action,false)}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let analyze = |fuel| moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), fuel);
    let report = analyze(100000).unwrap();
    assert_eq!(report.callable_field_exits.len(), 2);
    assert!(report.callable_field_exits[0].contains_key(&vec![0]));
    assert!(
        report.callable_field_exits[1].is_empty(),
        "a callable type is not evidence of its code identity"
    );
    assert!(analyze(0).is_err());
    assert!(analyze(100000).is_ok());
    let dump = report.to_string();
    assert!(dump.contains("(callable-field-exits (fields (leaf (path 0)"));
    println!("AGGREGATE_RESULT_DUMP={dump}");
}

#[test]
fn aggregate_callable_parameter_premises_validate_each_field() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let package = compile(
        "struct Pair {first:fn(bool)->bool,flag:bool,second:fn(bool)->bool} fn negate(value:bool)->bool {not value} fn identity(value:bool)->bool {value} fn apply(pair:Pair)->bool {pair.first(false) and pair.second(true)}",
    );
    let apply = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let input = |name: &str, fields: Vec<usize>| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        CallableParameter {
            choices: vec![],
            enum_captures: Default::default(),
            parameter: 0,
            fields,
            definition: nir::program::Expr {
                source: function.body.source,
                ty: function.signature.clone(),
                kind: nir::program::ExprKind::Function(function.declaration),
            },
            alternatives: vec![],
            captures: Default::default(),
        }
    };
    let inputs = vec![input("negate", vec![0]), input("identity", vec![2])];
    let analyze = |inputs: &[CallableParameter], fuel| {
        moves::analyze_in_package_with_inputs(
            &package,
            apply,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            inputs,
        )
    };
    let report = analyze(&inputs, 100000).unwrap();
    assert_eq!(report.callable_parameters, inputs);
    assert!(report.deferred_calls.is_empty());
    assert!(report.to_string().contains("(fields 2)"));
    let mut duplicate = inputs.clone();
    duplicate.push(inputs[0].clone());
    assert!(
        analyze(&duplicate, 100000)
            .unwrap_err()
            .reason
            .contains("duplicate callable parameter")
    );
    let mut bad_path = inputs.clone();
    bad_path[0].fields = vec![99];
    assert!(
        analyze(&bad_path, 100000)
            .unwrap_err()
            .reason
            .contains("out of bounds")
    );
    let mut scalar_path = inputs.clone();
    scalar_path[0].fields = vec![1];
    assert!(
        analyze(&scalar_path, 100000)
            .unwrap_err()
            .reason
            .contains("type disagrees")
    );
    assert!(analyze(&inputs, 0).is_err());
    assert!(analyze(&inputs, 100000).is_ok());
}

#[test]
fn enum_callable_parameter_premises_are_guarded_and_typed() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let package = compile(
        "enum Choice where T {empty,branch(bool,T),other(fn(bool)->bool)} fn negate(value:bool)->bool {not value} fn identity(value:bool)->bool {value} fn apply(value:Choice<fn(bool)->bool>)->bool {value match {.empty=>false,.branch(flag,action)=>action(flag),.other(action)=>action(true)}} fn extract(value:Choice<fn(bool)->bool>)->fn(bool)->bool {value match {.empty=>identity,.branch(_,action)=>action,.other(action)=>action}}",
    );
    let apply = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let input = |name: &str, fields: Vec<usize>| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        CallableParameter {
            choices: vec![],
            enum_captures: Default::default(),
            parameter: 0,
            fields,
            definition: nir::program::Expr {
                source: function.body.source,
                ty: function.signature.clone(),
                kind: nir::program::ExprKind::Function(function.declaration),
            },
            alternatives: vec![],
            captures: Default::default(),
        }
    };
    let paths =
        moves::callable_parameter_paths(&package, &apply.signature, &mut 10000, apply.body.source)
            .unwrap();
    assert_eq!(
        paths,
        vec![(0, vec![usize::MAX, 1, 1]), (0, vec![usize::MAX, 2, 0])]
    );
    let inputs = vec![
        input("negate", paths[0].1.clone()),
        input("identity", paths[1].1.clone()),
    ];
    let analyze = |inputs: &[CallableParameter], fuel| {
        moves::analyze_in_package_with_inputs(
            &package,
            apply,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            inputs,
        )
    };
    let report = analyze(&inputs, 100000).unwrap();
    assert_eq!(report.callable_parameters, inputs);
    assert!(report.deferred_calls.is_empty());
    assert!(report.to_string().contains("(fields (variant 1) 1)"));
    // The remaining variant cannot inherit the first variant's code identity.
    let extract = package
        .functions
        .iter()
        .find(|f| f.name == "extract")
        .unwrap();
    let returned = |premises: &[CallableParameter]| {
        moves::analyze_in_package_with_inputs(
            &package,
            extract,
            |_| Ok(Delivery::Copy),
            100000,
            None,
            &[],
            premises,
        )
        .unwrap()
    };
    assert!(
        returned(&inputs)
            .callable_evidence_exits
            .iter()
            .all(Option::is_some)
    );
    let partial = returned(&inputs[..1]);
    assert_eq!(partial.callable_evidence_exits.len(), 1);
    assert!(partial.callable_evidence_exits[0].is_none());
    for path in [
        vec![usize::MAX, 0, 0],
        vec![usize::MAX, 99, 0],
        vec![usize::MAX, 1, 99],
        vec![1],
    ] {
        let invalid = vec![input("negate", path)];
        assert!(analyze(&invalid, 100000).is_err());
    }
    let scalar = vec![input("negate", vec![usize::MAX, 1, 0])];
    assert!(
        analyze(&scalar, 100000)
            .unwrap_err()
            .reason
            .contains("type disagrees")
    );
    let duplicate = vec![inputs[0].clone(), inputs[0].clone()];
    assert!(
        analyze(&duplicate, 100000)
            .unwrap_err()
            .reason
            .contains("duplicate callable parameter")
    );
    assert!(analyze(&inputs, 0).is_err());
    assert!(analyze(&inputs, 100000).is_ok());
}

#[test]
fn enum_domain_joins_do_not_invent_missing_callback_provenance() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "enum Choice {left(fn(bool)->bool),right(fn(bool)->bool)} fn first(value:bool)->bool {not value} fn second(value:bool)->bool {value} fn different(flag:bool,unknown:fn(bool)->bool)->Choice {if flag {Choice.left(first)} else {Choice.right(unknown)}} fn same(flag:bool,unknown:fn(bool)->bool)->Choice {if flag {Choice.left(first)} else {Choice.left(unknown)}} fn uncertain(flag:bool,unknown:Choice)->Choice {if flag {Choice.left(first)} else {unknown}} fn overwritten(flag:bool,unknown:fn(bool)->bool)->Choice {var value=Choice.left(first);if flag {value=Choice.right(second);} else {value=Choice.left(unknown);}value} fn untracked(flag:bool,producer:fn(Choice)->Choice)->Choice {if flag {Choice.left(first)} else {producer(Choice.right(second))}}",
    );
    for (name, expected) in [
        ("different", vec![vec![usize::MAX, 0, 0]]),
        ("same", vec![]),
        ("uncertain", vec![]),
        ("untracked", vec![]),
        ("overwritten", vec![vec![usize::MAX, 1, 0]]),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let analyze =
            |fuel| moves::analyze_in_package(&package, function, |_| Ok(Delivery::Copy), fuel);
        let report = analyze(100000).unwrap();
        assert_eq!(report.callable_field_exits.len(), 1, "{name}");
        assert_eq!(
            report.callable_field_exits[0]
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            expected,
            "{name}"
        );
        assert!(analyze(0).is_err());
        assert!(analyze(100000).is_ok());
    }
}

#[test]
fn enum_return_domains_survive_postconditions_and_exclude_failures() {
    use elaboration::verification::moves::{self, Delivery};
    let mut package = compile(
        "enum Choice {left(fn(bool)->bool),right(fn(bool)->bool)} fn first(value:bool)->bool {not value} fn second(value:bool)->bool {value} fn make(flag:bool)->Choice {if flag {return Choice.left(first);}Choice.right(second)} fn forward(flag:bool)->Choice {make(flag)} fn reject()->Choice {assumes false;Choice.left(first)}",
    );
    let make = package
        .functions
        .iter_mut()
        .find(|f| f.name == "make")
        .unwrap();
    make.postconditions.push(nir::program::Expr {
        source: make.body.source,
        ty: nir::Term::Primitive(nir::Primitive::Bool),
        kind: nir::program::ExprKind::Literal(nir::program::Literal::Bool(true)),
    });
    let analyze = |name: &str, fuel| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        moves::analyze_in_package(&package, function, |_| Ok(Delivery::Copy), fuel)
    };
    let report = analyze("make", 100000).unwrap();
    assert_eq!(report.enum_variant_exits.len(), 2);
    assert_eq!(
        report.enum_variant_exits[0].get(&vec![]),
        Some(&std::collections::BTreeSet::from([0]))
    );
    assert_eq!(
        report.enum_variant_exits[1].get(&vec![]),
        Some(&std::collections::BTreeSet::from([1]))
    );
    let forward = analyze("forward", 100000).unwrap();
    assert_eq!(forward.enum_variant_exits.len(), 1);
    assert_eq!(
        forward.enum_variant_exits[0].get(&vec![]),
        Some(&std::collections::BTreeSet::from([0, 1]))
    );
    assert_eq!(
        forward.callable_field_exits[0]
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![vec![usize::MAX, 0, 0], vec![usize::MAX, 1, 0]]
    );
    let rejected = analyze("reject", 100000).unwrap();
    assert!(!rejected.failed_exits.is_empty());
    assert!(rejected.enum_variant_exits.is_empty());
    assert!(rejected.callable_field_exits.is_empty());
    assert!(analyze("forward", 0).is_err());
    assert!(analyze("forward", 100000).is_ok());
    let dump = forward.to_string();
    assert!(
        dump.contains("(enum-variant-exits (domains (domain (path) (tags 0 1))))"),
        "{dump}"
    );
    println!("ENUM_RETURN_DUMP={dump}");
}

#[test]
fn enum_deferred_field_calls_do_not_reuse_argument_domains() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "enum Choice {left(fn(bool)->bool),right(fn(bool)->bool)} struct Producer {generate:fn(Choice)->Choice} fn second(value:bool)->bool {value} fn probe(producer:Producer) {let callback=|| producer.generate(Choice.right(second));}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "probe")
        .unwrap();
    let report =
        moves::analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 100000).unwrap();
    let construction = report
        .closures
        .iter()
        .find(|closure| !closure.invoked)
        .unwrap();
    assert!(!construction.invocation.deferred_calls.is_empty());
    assert_eq!(construction.invocation.enum_variant_exits.len(), 1);
    assert!(construction.invocation.enum_variant_exits[0].is_empty());
    assert!(construction.invocation.callable_field_exits[0].is_empty());
}

#[test]
fn enum_parameter_tag_premises_validate_paths_and_reachable_arms() {
    use elaboration::verification::moves::{self, Delivery, EnumParameter};
    use std::collections::{BTreeMap, BTreeSet};
    let package = compile(
        "enum Choice {none,some(fn(bool)->bool)} struct Packet {flag:bool,value:Choice} enum Outer {one(Choice),two} fn inspect(packet:Packet)->bool {packet.value match {.none=>false,.some(action)=>action(true)}} fn nested(value:Outer)->bool {value match {.one(inner)=>inner match {.none=>false,.some(action)=>action(true)},.two=>false}}",
    );
    let input = |path: Vec<usize>, tags: &[usize]| EnumParameter {
        parameter: 0,
        domains: BTreeMap::from([(path, tags.iter().copied().collect())]),
    };
    let analyze = |name: &str, inputs: &[EnumParameter], fuel| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        moves::analyze_in_package_with_context(
            &package,
            function,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            &[],
            inputs,
        )
    };
    let inputs = vec![input(vec![1], &[0])];
    let report = analyze("inspect", &inputs, 100000).unwrap();
    assert_eq!(report.enum_parameters, inputs);
    assert_eq!(report.matches.len(), 1);
    assert_eq!(report.matches[0].arms.len(), 1);
    assert!(matches!(
        report.matches[0].arms[0].pattern,
        nir::program::Pattern::Variant { index: 0, .. }
    ));
    assert_eq!(
        analyze("inspect", &[], 100000).unwrap().matches[0]
            .arms
            .len(),
        2
    );
    assert!(
        report
            .to_string()
            .contains("(enum-parameter (parameter 0) (domains (domain (path 1) (tags 0))))")
    );
    for bad in [
        input(vec![0], &[0]),
        input(vec![99], &[0]),
        input(vec![1], &[2]),
        input(vec![1], &[]),
        input(vec![1, 0], &[0]),
    ] {
        assert!(analyze("inspect", &[bad], 100000).is_err());
    }
    let mut missing = inputs[0].clone();
    missing.parameter = 99;
    assert!(analyze("inspect", &[missing], 100000).is_err());
    assert!(
        analyze("inspect", &[inputs[0].clone(), inputs[0].clone()], 100000)
            .unwrap_err()
            .reason
            .contains("duplicate enum parameter")
    );
    assert!(
        analyze(
            "inspect",
            &[EnumParameter {
                parameter: 0,
                domains: BTreeMap::new()
            }],
            100000
        )
        .is_err()
    );
    let contradictory = EnumParameter {
        parameter: 0,
        domains: BTreeMap::from([
            (vec![], BTreeSet::from([1])),
            (vec![usize::MAX, 0, 0], BTreeSet::from([0])),
        ]),
    };
    assert!(
        analyze("nested", &[contradictory], 100000)
            .unwrap_err()
            .reason
            .contains("contradictory")
    );
    assert!(analyze("inspect", &inputs, 0).is_err());
    assert!(analyze("inspect", &inputs, 100000).is_ok());
}

#[test]
fn captured_enum_domain_evidence_validates_capture_identity_and_layout() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    use std::collections::{BTreeMap, BTreeSet};
    let package = compile(
        "enum Choice {none,some(fn(bool)->bool)} struct Packet {flag:bool,value:Choice} typealias Callback=%{packet:Packet} fn()->bool; fn make()->Callback {let packet=Packet {flag:false,value:Choice.none};|| packet.value match {.none=>false,.some(action)=>action(true)}} fn invoke(callback:Callback)->bool {callback()} fn plain()->bool {false} fn run(callback:fn()->bool)->bool {callback()}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let produced =
        moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let evidence = produced.callable_evidence_exits[0].as_ref().unwrap();
    assert_eq!(evidence.enum_captures.len(), 1);
    let root = *evidence.enum_captures.keys().next().unwrap();
    assert_eq!(
        evidence.enum_captures[&root],
        BTreeMap::from([(vec![1], BTreeSet::from([0]))])
    );
    let input = CallableParameter {
        choices: evidence.choices.clone(),
        parameter: 0,
        fields: vec![],
        definition: evidence.definition.clone(),
        alternatives: evidence.alternatives.clone(),
        captures: evidence.captures.clone(),
        enum_captures: evidence.enum_captures.clone(),
    };
    let analyze = |name: &str, input: &CallableParameter, fuel| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        moves::analyze_in_package_with_inputs(
            &package,
            function,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            std::slice::from_ref(input),
        )
    };
    let report = analyze("invoke", &input, 100000).unwrap();
    assert_eq!(report.callable_parameters, vec![input.clone()]);
    assert_eq!(report.closures[0].invocation.matches[0].arms.len(), 1);
    let mut unknown = input.clone();
    unknown.enum_captures.clear();
    assert_eq!(
        analyze("invoke", &unknown, 100000).unwrap().closures[0]
            .invocation
            .matches[0]
            .arms
            .len(),
        2
    );
    let mut missing = input.clone();
    missing.enum_captures = BTreeMap::from([(
        elaboration::verification::PlaceId {
            owner: u64::MAX,
            local: 0,
        },
        evidence.enum_captures[&root].clone(),
    )]);
    assert!(
        analyze("invoke", &missing, 100000)
            .unwrap_err()
            .reason
            .contains("unknown capture")
    );
    for domains in [
        BTreeMap::from([(vec![0], BTreeSet::from([0]))]),
        BTreeMap::from([(vec![1], BTreeSet::from([99]))]),
        BTreeMap::from([(vec![1], BTreeSet::new())]),
        BTreeMap::new(),
    ] {
        let mut invalid = input.clone();
        invalid.enum_captures.insert(root, domains);
        assert!(analyze("invoke", &invalid, 100000).is_err());
    }
    let plain = package
        .functions
        .iter()
        .find(|f| f.name == "plain")
        .unwrap();
    let mut named = input.clone();
    named.definition = nir::program::Expr {
        source: plain.body.source,
        ty: plain.signature.clone(),
        kind: nir::program::ExprKind::Function(plain.declaration),
    };
    assert!(
        analyze("run", &named, 100000)
            .unwrap_err()
            .reason
            .contains("named callable cannot carry enum capture")
    );
    assert!(analyze("invoke", &input, 0).is_err());
    assert!(analyze("invoke", &input, 100000).is_ok());
    let dump = report.to_string();
    assert!(dump.contains("(enum-captures (capture (owner"));
    assert!(dump.contains("(domain (path 1) (tags 0))"));
    println!("CAPTURED_ENUM_DUMP={dump}");
}

#[test]
fn distinct_closure_input_evidence_validates_every_candidate_and_recovers() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let package = compile(
        "typealias Callback=%{action:fn(bool)->bool} fn(bool)->bool; fn first(value:bool)->bool {value} fn second(value:bool)->bool {not value} fn make(flag:bool)->Callback {let left=first;let right=second;if flag {|value:bool|left(value)} else {|value:bool|right(value)}} fn apply(callback:Callback)->bool {callback(true)}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let apply = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let report = moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let evidence = report.callable_evidence_exits[0].as_ref().unwrap();
    assert_eq!(evidence.choices.len(), 1);
    let input = CallableParameter {
        parameter: 0,
        fields: vec![],
        definition: evidence.definition.clone(),
        alternatives: evidence.alternatives.clone(),
        captures: evidence.captures.clone(),
        enum_captures: evidence.enum_captures.clone(),
        choices: evidence.choices.clone(),
    };
    let analyze = |input: &CallableParameter, fuel| {
        moves::analyze_in_package_with_inputs(
            &package,
            apply,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            std::slice::from_ref(input),
        )
    };
    let valid = analyze(&input, 100000).unwrap();
    assert!(valid.deferred_calls.is_empty());
    assert_eq!(valid.closures.len(), 2);
    let mut bad = input.clone();
    bad.choices[0].definition.ty = nir::Term::Primitive(nir::Primitive::Bool);
    assert!(
        analyze(&bad, 100000)
            .unwrap_err()
            .reason
            .contains("type disagrees")
    );
    let mut bad = input.clone();
    bad.choices.push(bad.choices[0].clone());
    assert!(
        analyze(&bad, 100000)
            .unwrap_err()
            .reason
            .contains("duplicate closure")
    );
    let mut bad = input.clone();
    bad.choices[0].choices.push(evidence.clone());
    assert!(
        analyze(&bad, 100000)
            .unwrap_err()
            .reason
            .contains("flat callable")
    );
    let mut bad = input.clone();
    // The other body's capture binding is not a valid premise for this body.
    bad.choices[0].captures = input.captures.clone();
    assert!(
        analyze(&bad, 100000)
            .unwrap_err()
            .reason
            .contains("unknown capture")
    );
    let mut bad = input.clone();
    let place = bad.choices[0].captures.keys().next().unwrap().clone();
    let mut invalid = place.clone();
    invalid.fields.push(99);
    let captured = bad.choices[0].captures.remove(&place).unwrap();
    bad.choices[0].captures.insert(invalid, captured);
    assert!(analyze(&bad, 100000).is_err());
    assert!(analyze(&input, 0).is_err());
    assert!(analyze(&input, 100000).is_ok());
    let dump = input.to_string();
    assert!(dump.contains("(choices (callable-evidence"));
    println!("CLOSURE_CHOICES_DUMP={dump}");
}

#[test]
fn mixed_callable_input_evidence_validates_named_candidates_independently() {
    use elaboration::verification::moves::{self, CallableParameter, Delivery};
    let package = compile(
        "fn first(value:bool)->bool {value} fn second(value:bool)->bool {not value} fn make(flag:bool)->fn(bool)->bool {if flag {if flag {first} else {second}} else {|value:bool|first(value)}} fn apply(action:fn(bool)->bool)->bool {action(true)}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let apply = package
        .functions
        .iter()
        .find(|f| f.name == "apply")
        .unwrap();
    let report = moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let evidence = report.callable_evidence_exits[0].as_ref().unwrap();
    assert_eq!(evidence.choices.len(), 2);
    assert!(matches!(
        evidence.definition.kind,
        nir::program::ExprKind::Closure { .. }
    ));
    let input = CallableParameter {
        parameter: 0,
        fields: vec![],
        definition: evidence.definition.clone(),
        alternatives: vec![],
        captures: evidence.captures.clone(),
        enum_captures: evidence.enum_captures.clone(),
        choices: evidence.choices.clone(),
    };
    let analyze = |input: &CallableParameter, fuel| {
        moves::analyze_in_package_with_inputs(
            &package,
            apply,
            |_| Ok(Delivery::Copy),
            fuel,
            None,
            &[],
            std::slice::from_ref(input),
        )
    };
    assert!(analyze(&input, 100000).unwrap().deferred_calls.is_empty());
    let mut bad = input.clone();
    let mut duplicate = bad.choices[0].clone();
    duplicate.definition.source.start += 1;
    duplicate.definition.source.end += 1;
    bad.choices.push(duplicate);
    assert!(
        analyze(&bad, 100000)
            .unwrap_err()
            .reason
            .contains("duplicate"),
        "named identity must ignore reference spans"
    );
    let mut bad = input.clone();
    bad.choices[0].definition.ty = nir::Term::Primitive(nir::Primitive::Bool);
    assert!(
        analyze(&bad, 100000)
            .unwrap_err()
            .reason
            .contains("type disagrees")
    );
    let mut bad = input.clone();
    bad.choices[0].choices.push(evidence.clone());
    assert!(
        analyze(&bad, 100000)
            .unwrap_err()
            .reason
            .contains("flat callable")
    );
    assert!(analyze(&input, 0).is_err());
    assert!(analyze(&input, 100000).is_ok());
    println!("MIXED_CALLABLE_DUMP={input}");
}

#[test]
fn closure_memory_footprint_covers_early_returns_calls_and_budget_recovery() {
    use elaboration::verification::{
        memory,
        moves::{self, Delivery},
    };
    let package = compile(
        "fn helper(value:*bool)->*bool {value.* = true;value} fn make()->fn(*bool,*bool,bool)->*bool {|left:*bool,right:*bool,flag:bool| {if flag {return helper(left);}right}} fn make_captured()->%{flag:bool} fn(*bool)->*bool {let flag=true;|value:*bool| {if flag {return value;}value}}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let report = moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let definition = &report.callable_evidence_exits[0]
        .as_ref()
        .unwrap()
        .definition;
    let summary = memory::infer_closure(&package, definition, &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 0 && access.write)
    );
    assert_eq!(
        summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|target| target.parameter)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([0, 1])
    );
    assert!(
        memory::infer_closure(&package, definition, &mut 0)
            .unwrap_err()
            .contains("budget")
    );
    assert_eq!(
        memory::infer_closure(&package, definition, &mut 100000).unwrap(),
        summary
    );
    let mut malformed = definition.clone();
    if let nir::program::ExprKind::Closure { parameters, .. } = &mut malformed.kind {
        parameters[1] = parameters[0];
    }
    assert!(
        memory::infer_closure(&package, &malformed, &mut 100000)
            .unwrap_err()
            .contains("signature mismatch")
    );
    let make = package
        .functions
        .iter()
        .find(|f| f.name == "make_captured")
        .unwrap();
    let report = moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let captured = &report.callable_evidence_exits[0]
        .as_ref()
        .unwrap()
        .definition;
    let captured_summary = memory::infer_closure(&package, captured, &mut 100000).unwrap();
    assert_eq!(captured_summary.result_targets.as_ref().unwrap().len(), 1);
    let mut pointer_capture = captured.clone();
    let pointer = nir::Term::Pointer(Box::new(nir::Term::Primitive(nir::Primitive::Bool)));
    if let nir::program::ExprKind::Closure { captures, .. } = &mut pointer_capture.kind {
        captures[0].ty = pointer.clone();
    }
    if let nir::Term::Closure { environment, .. } = &mut pointer_capture.ty {
        environment[0] = pointer;
    }
    assert!(
        memory::infer_closure(&package, &pointer_capture, &mut 100000)
            .unwrap_err()
            .contains("environment memory contract")
    );
    let mut wrong_layout = captured.clone();
    if let nir::Term::Closure { environment, .. } = &mut wrong_layout.ty {
        environment.clear();
    }
    assert!(
        memory::infer_closure(&package, &wrong_layout, &mut 100000)
            .unwrap_err()
            .contains("layout mismatch")
    );
    println!("CLOSURE_MEMORY_DUMP={summary}");
}

#[test]
fn captured_callee_memory_premises_validate_identity_shape_and_all_candidates() {
    use elaboration::verification::{
        memory,
        moves::{self, Delivery},
    };
    use std::collections::{BTreeMap, BTreeSet};
    let package = compile(
        "fn first(a:*bool,b:*bool)->*bool {a.* = true;a} fn second(a:*bool,b:*bool)->*bool {b.* = false;b} fn wrong(value:bool)->bool {value} fn make(flag:bool)->%{action:fn(*bool,*bool)->*bool} fn(*bool,*bool)->*bool {let action=if flag {first} else {second};|a:*bool,b:*bool|action(a,b)}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let report = moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let definition = &report.callable_evidence_exits[0]
        .as_ref()
        .unwrap()
        .definition;
    let nir::program::ExprKind::Closure {
        captures,
        parameters,
        ..
    } = &definition.kind
    else {
        panic!("expected closure")
    };
    let binding = captures[0].binding;
    let key = |name: &str| iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    let input = BTreeMap::from([(
        binding,
        memory::CallableResult::Candidates(vec![key("first"), key("second")]),
    )]);
    let infer = |input: &BTreeMap<_, _>, fuel| {
        memory::infer_closure_with_callees(&package, definition, input, &mut { fuel })
    };
    let summary = infer(&input, 100000).unwrap();
    assert_eq!(
        summary
            .accesses
            .iter()
            .filter(|access| access.write)
            .map(|access| access.target.parameter)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([0, 1])
    );
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
        memory::infer_closure(&package, definition, &mut 100000)
            .unwrap_err()
            .contains("richer memory contract")
    );
    let bad = BTreeMap::from([(binding, memory::CallableResult::Known(key("wrong")))]);
    assert!(
        infer(&bad, 100000)
            .unwrap_err()
            .contains("signature disagrees")
    );
    let bad = BTreeMap::from([(parameters[0], memory::CallableResult::Known(key("first")))]);
    assert!(infer(&bad, 100000).unwrap_err().contains("unknown capture"));
    let bad = BTreeMap::from([(
        binding,
        memory::CallableResult::Candidates(vec![key("first"), key("first")]),
    )]);
    assert!(infer(&bad, 100000).unwrap_err().contains("duplicate"));
    let bad = BTreeMap::from([(binding, memory::CallableResult::Unknown)]);
    assert!(
        infer(&bad, 100000)
            .unwrap_err()
            .contains("named code identities")
    );
    let bad = BTreeMap::from([(
        binding,
        memory::CallableResult::Forwarded {
            named: vec![key("first")],
            inputs: vec![memory::CallableInput {
                parameter: 0,
                fields: vec![],
            }],
        },
    )]);
    assert!(
        infer(&bad, 100000)
            .unwrap_err()
            .contains("named code identities")
    );
    let mut missing = key("first");
    missing.declaration.local = 999999;
    let bad = BTreeMap::from([(binding, memory::CallableResult::Known(missing))]);
    assert!(infer(&bad, 100000).unwrap_err().contains("missing"));
    assert!(infer(&input, 0).is_err());
    assert_eq!(infer(&input, 100000).unwrap(), summary);
    println!("CAPTURED_CALLEE_MEMORY_DUMP={summary}");
}

#[test]
fn captured_memory_callable_paths_validate_layout_and_preserve_unknowns() {
    use elaboration::verification::{
        memory,
        moves::{self, Delivery},
    };
    use std::collections::BTreeMap;
    let package = tuple_support::compile_with_tuple(
        "struct Ops {pair:Tuple<fn(*bool)->*bool,bool>} fn set(value:*bool)->*bool {value.* = true;value} fn make()->%{ops:Ops} fn(*bool)->*bool {let ops=Ops {pair:(set,false)};|value:*bool| {let (action,_)=ops.pair;action(value)}}",
    );
    let make = package.functions.iter().find(|f| f.name == "make").unwrap();
    let report = moves::analyze_in_package(&package, make, |_| Ok(Delivery::Copy), 100000).unwrap();
    let definition = &report.callable_evidence_exits[0]
        .as_ref()
        .unwrap()
        .definition;
    let nir::program::ExprKind::Closure { captures, .. } = &definition.kind else {
        panic!("expected closure")
    };
    let place = memory::CallablePlace {
        binding: captures[0].binding,
        fields: vec![0, 0],
    };
    let key = memory::CallableResult::Known(iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == "set")
            .unwrap()
            .declaration,
        arguments: vec![],
    });
    let input = BTreeMap::from([(place.clone(), key.clone())]);
    let infer = |input: &BTreeMap<_, _>, fuel| {
        memory::infer_closure_with_fields(&package, definition, input, &mut { fuel })
    };
    let summary = infer(&input, 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.write && access.target.parameter == 0)
    );
    assert!(
        infer(&BTreeMap::new(), 100000)
            .unwrap_err()
            .contains("richer memory contract")
    );
    let mut invalid = place.clone();
    invalid.fields[1] = 99;
    assert!(
        infer(&BTreeMap::from([(invalid, key.clone())]), 100000)
            .unwrap_err()
            .contains("out of bounds")
    );
    let mut invalid = place.clone();
    invalid.fields[1] = 1;
    assert!(
        infer(&BTreeMap::from([(invalid, key)]), 100000)
            .unwrap_err()
            .contains("named code identities")
    );
    assert!(infer(&input, 0).is_err());
    assert_eq!(infer(&input, 100000).unwrap(), summary);
    println!("MEMORY_CALLABLE_FIELD_DUMP=(field-footprint {place} {summary})");
}

#[test]
fn recursive_aggregate_memory_results_keep_bottom_distinct_from_unknown() {
    use elaboration::verification::memory::{self, CallableResult};
    let package = tuple_support::compile_with_tuple(
        "struct Ops {pair:Tuple<fn(*bool)->*bool,bool>} fn first(value:*bool)->*bool {value} fn second(value:*bool)->*bool {value.* = true;value} fn left(flag:bool)->Ops {if flag {return Ops {pair:(first,true)};}right(true)} fn right(flag:bool)->Ops {if flag {return Ops {pair:(second,false)};}left(true)} fn unproven(action:fn(*bool)->*bool)->Ops {Ops {pair:(action,false)}} fn anon_result()->Ops {Ops {pair:(|value:*bool|value,false)}} fn cycle()->Ops {cycle()}",
    );
    let infer = |name: &str, fuel| {
        memory::infer_in_package(
            &package,
            package
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .declaration,
            &[],
            &mut { fuel },
        )
    };
    let summary = infer("left", 100000).unwrap();
    let CallableResult::Candidates(keys) = &summary.callable_fields[&vec![0, 0]] else {
        panic!("{summary}")
    };
    assert_eq!(keys.len(), 2);
    for name in ["first", "second"] {
        assert!(keys.iter().any(|key| {
            key.declaration
                == package
                    .functions
                    .iter()
                    .find(|f| f.name == name)
                    .unwrap()
                    .declaration
        }));
    }
    assert_eq!(summary.callable_fields.len(), 1);
    assert!(
        matches!(&infer("unproven",100000).unwrap().callable_fields[&vec![0,0]],CallableResult::Forwarded {named,inputs} if named.is_empty() && inputs.len()==1 && inputs[0].parameter==0 && inputs[0].fields.is_empty())
    );
    assert_eq!(
        infer("anon_result", 100000).unwrap().callable_fields[&vec![0, 0]],
        CallableResult::Unknown
    );
    assert_eq!(
        infer("cycle", 100000).unwrap().callable_fields[&vec![0, 0]],
        CallableResult::NoReturn
    );
    for fuel in [0, 1, 4] {
        assert!(infer("left", fuel).is_err());
        assert_eq!(infer("left", 100000).unwrap(), summary);
    }
    println!("AGGREGATE_MEMORY_SUMMARY_DUMP={summary}");
}

#[test]
fn callable_input_origins_compose_without_reinterpreting_the_callers_frame() {
    use elaboration::verification::memory::{self, CallableInput, CallableResult};
    let package = compile(
        "struct Ops {action:fn(*bool)->*bool} fn named(value:*bool)->*bool {value} fn pack(action:fn(*bool)->*bool)->Ops {Ops {action:action}} fn repack(ops:Ops)->Ops {pack(ops.action)} fn transitive(ops:Ops)->Ops {repack(ops)} fn mixed(ops:Ops,flag:bool)->Ops {if flag {ops} else {Ops {action:named}}} fn relay(ops:Ops,stop:bool)->Ops {if stop {return ops;}relay(ops,true)} fn scalar(action:fn(*bool)->*bool)->fn(*bool)->*bool {action}",
    );
    let infer = |name: &str, fuel| {
        memory::infer_in_package(
            &package,
            package
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .declaration,
            &[],
            &mut { fuel },
        )
    };
    let input = CallableInput {
        parameter: 0,
        fields: vec![0],
    };
    let expected = CallableResult::Forwarded {
        named: vec![],
        inputs: vec![input.clone()],
    };
    for name in ["repack", "transitive", "relay"] {
        assert_eq!(
            infer(name, 100000).unwrap().callable_fields[&vec![0]],
            expected,
            "{name}"
        );
    }
    let mixed = infer("mixed", 100000).unwrap();
    assert!(
        matches!(&mixed.callable_fields[&vec![0]],CallableResult::Forwarded {named,inputs} if named.len()==1 && inputs==&vec![input])
    );
    assert_eq!(
        infer("scalar", 100000).unwrap().callable_result,
        CallableResult::Forwarded {
            named: vec![],
            inputs: vec![CallableInput {
                parameter: 0,
                fields: vec![]
            }]
        }
    );
    assert!(infer("relay", 0).is_err());
    assert_eq!(
        infer("relay", 100000).unwrap().callable_fields[&vec![0]],
        expected
    );
    println!("FORWARDED_CALLABLE_MEMORY_DUMP={mixed}");
}

#[test]
fn contextual_memory_worklists_separate_callable_inputs_and_validate_premises() {
    use elaboration::verification::memory::{self, CallableInput, CallableResult};
    use std::collections::BTreeMap;
    let package = compile(
        "fn first(a:*bool,b:*bool)->*bool {let seen=a.*;a} fn second(a:*bool,b:*bool)->*bool {b.* = false;b} fn wrong(value:bool)->bool {value} fn apply(action:fn(*bool,*bool)->*bool,a:*bool,b:*bool)->*bool {action(a,b)} fn both(a:*bool,b:*bool)->*bool {let result=apply(first,a,b);apply(second,a,b);result} fn rotate(action:fn(*bool,*bool)->*bool,a:*bool,b:*bool,stop:bool)->*bool {if stop {return action(a,b);}rotate(second,a,b,true)}",
    );
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .declaration
    };
    let key = |name: &str| iir::InstanceKey {
        declaration: id(name),
        arguments: vec![],
    };
    let input = CallableInput {
        parameter: 0,
        fields: vec![],
    };
    let first = CallableResult::Known(key("first"));
    let second = CallableResult::Known(key("second"));
    let infer = |name: &str, inputs: &BTreeMap<CallableInput, CallableResult>, fuel| {
        memory::infer_with_inputs(&package, id(name), &[], inputs, &mut { fuel })
    };
    for (value, target, write) in [(first.clone(), 1, false), (second.clone(), 2, true)] {
        let inputs = BTreeMap::from([(input.clone(), value)]);
        let summary = infer("apply", &inputs, 100000).unwrap();
        assert_eq!(summary.callable_inputs, inputs);
        assert_eq!(summary.result_targets.as_ref().unwrap().len(), 1);
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .first()
                .unwrap()
                .parameter,
            target
        );
        assert_eq!(summary.accesses.len(), 1);
        assert_eq!(summary.accesses[0].target.parameter, target);
        assert_eq!(summary.accesses[0].write, write);
    }
    let combined = BTreeMap::from([(
        input.clone(),
        CallableResult::Candidates(vec![key("first"), key("second")]),
    )]);
    let summary = infer("apply", &combined, 100000).unwrap();
    assert_eq!(summary.accesses.len(), 2);
    assert_eq!(summary.result_targets.as_ref().unwrap().len(), 2);
    let both = infer("both", &BTreeMap::new(), 100000).unwrap();
    assert_eq!(both.accesses.len(), 2);
    assert_eq!(both.result_targets.as_ref().unwrap().len(), 1);
    assert_eq!(
        both.result_targets
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .parameter,
        0
    );
    let rotated = infer(
        "rotate",
        &BTreeMap::from([(input.clone(), first.clone())]),
        100000,
    )
    .unwrap();
    assert_eq!(rotated.result_targets.as_ref().unwrap().len(), 2);
    for invalid in [
        BTreeMap::new(),
        BTreeMap::from([(
            CallableInput {
                parameter: 8,
                fields: vec![],
            },
            first.clone(),
        )]),
        BTreeMap::from([(
            CallableInput {
                parameter: 0,
                fields: vec![0],
            },
            first.clone(),
        )]),
        BTreeMap::from([(
            CallableInput {
                parameter: 1,
                fields: vec![],
            },
            first.clone(),
        )]),
        BTreeMap::from([(input.clone(), CallableResult::Known(key("wrong")))]),
        BTreeMap::from([(input.clone(), CallableResult::Candidates(vec![]))]),
        BTreeMap::from([(
            input.clone(),
            CallableResult::Candidates(vec![key("first"), key("first")]),
        )]),
        BTreeMap::from([(input.clone(), CallableResult::NoReturn)]),
        BTreeMap::from([(input.clone(), CallableResult::Unknown)]),
        BTreeMap::from([(
            input.clone(),
            CallableResult::Forwarded {
                named: vec![key("first")],
                inputs: vec![input.clone()],
            },
        )]),
    ] {
        assert!(
            infer("apply", &invalid, 100000).is_err(),
            "invalid premises: {invalid:?}"
        );
    }
    assert!(infer("apply", &combined, 0).is_err());
    assert_eq!(infer("apply", &combined, 100000).unwrap(), summary);
    println!("CONTEXTUAL_MEMORY_DUMP={summary}");
}

#[test]
fn contextual_memory_rejects_malformed_direct_frames_before_argument_snapshots() {
    use elaboration::verification::memory;
    use nir::program::ExprKind as K;
    for duplicate in [false, true] {
        let mut package = compile(
            "fn action(a:*bool,b:*bool)->*bool {a} fn forward(a:*bool,b:*bool)->*bool {action(a,b)}",
        );
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.name == "forward")
            .unwrap();
        let K::Block {
            result: Some(tail), ..
        } = &mut function.body.kind
        else {
            panic!("block")
        };
        let K::Call {
            parameter_bindings, ..
        } = &mut tail.kind
        else {
            panic!("call")
        };
        if duplicate {
            parameter_bindings[1] = parameter_bindings[0];
        } else {
            parameter_bindings.clear();
        }
        let declaration = function.declaration;
        let error = memory::infer_in_package(&package, declaration, &[], &mut 100000).unwrap_err();
        assert!(
            error.contains("complete unique parameter binding frame"),
            "{error}"
        );
    }
}

#[test]
fn memory_field_pointer_context_reads_multiple_levels_without_changing_aliases() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:*bool} struct DeepCell {value:**bool}
        fn read(cell:*Cell)->bool {cell.value}
        fn accept(value:bool)->bool {value}
        fn run()->bool {var flag=true;let cell=Cell {value:flag.ref};let pointer=cell.ref;
            let snapshot:bool=pointer.value;let raw=pointer.value;
            let middle=flag.ref;let deep=DeepCell {value:middle.ref};let deep_pointer=deep.ref;
            let deep_snapshot:bool=deep_pointer.value;let still_pointer:*bool=deep_pointer.value;
            flag=false;let after:bool=pointer.value;
            snapshot and deep_snapshot and not still_pointer.*
                and not after and not read(pointer) and not accept(pointer.value) and not raw.*.*}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn memory_indirect_field_footprints_follow_current_pointer_contents() {
    use comptime_interpretor::Value;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:*bool}
        fn read(cell:*Cell)->bool {cell.value}
        fn relay(cell:*Cell)->bool {read(cell)}
        fn deeper(slot:**Cell)->bool {let cell=slot.*;relay(cell)}
        fn write(cell:*Cell) {cell.value.*.* = true;}
        fn view(cell:*Cell)->*bool {cell.value}
        fn after(cell:*Cell,ready:bool)->bool {read(cell)}";
    let source=format!("{prefix} fn run()->bool {{let first=false;let second=false;
        var cell=Cell {{value:first.ref}};write(cell.ref);let old=view(cell.ref);
        cell.value=second.ref;let before=relay(cell.ref);write(cell.ref);
        old.* and not before and relay(cell.ref) and do {{let pointer=cell.ref;deeper(pointer.ref)}} and first and second}}");
    let package = compile(&source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
    let read = package.functions.iter().find(|f| f.name == "read").unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        read.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert!(
        summary
            .to_string()
            .contains("(through (load-pointer (fields"),
        "{summary}"
    );
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let first=false;let second=true;
        var cell=Cell {{value:first.ref}};after(cell.ref,do {{cell.value=second.ref;true}})}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn memory_indirect_field_footprints_reject_dead_targets_and_pointer_stores() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Item {flag:bool} struct Link {target:*Item}
        fn consume(value:Item) {}
        fn read(link:*Link)->bool {link.target.*.*.flag}
        fn rebind(link:*Link,other:*Item) {link.target.* = other;}";
    for (body, expected) in [
        (
            "let item=Item {flag:true};let link=Link {target:item.ref};consume(item);read(link.ref)",
            "unavailable",
        ),
        (
            "let first=Item {flag:true};let second=Item {flag:false};let link=Link {target:first.ref};rebind(link.ref,second.ref);true",
            "pointer store",
        ),
    ] {
        let package = compile(&format!("{prefix} fn run()->bool {{{body}}}"));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn checked_pointer_predicates_require_live_targets() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for moved in ["", "let moved=cell;"] {
        let package = compile(&format!(
            "struct Cell {{flag:bool}} fn run() {{let cell=Cell {{flag:true}};let pointer=cell.flag.ref;{moved} assumes pointer.*;}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
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
            100000,
        );
        if moved.is_empty() {
            result.unwrap();
        } else {
            assert!(result.unwrap_err().to_string().contains("unavailable"));
        }
    }
    let package = compile("fn checked(pointer:*bool) {assumes pointer.*;}");
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        package.functions[0].declaration,
        &[],
        &mut 100000,
    )
    .unwrap();
    assert_eq!(summary.accesses.len(), 1);
    assert!(summary.accesses[0].read_value);
    assert_eq!(summary.accesses[0].target.parameter, 0);
}

#[test]
fn memory_borrowed_pointer_premises_join_across_branches_and_loops() {
    use comptime_interpretor::Value;
    let prefix =
        "trait Duplicate {} trait Retire {fn drop(*self);} struct Pair {left:*bool,right:*bool}";
    for body in [
        "if flag {pair.left} else {pair.right}",
        "let chosen:*bool=if flag {pair.left} else {pair.right};chosen.*",
        "flag match {true=>pair.left,false=>pair.right}",
        "let first:bool=if flag {pair.left} else {pair.right};let second:bool=pair.left;first and second",
        "let read:fn(*Pair,bool)->bool = |p,flag| {if flag {p.left} else {p.right}};read(pair,flag)",
        "var again=flag;while again {let observed:bool=pair.left;again=false;}pair.right",
    ] {
        for flag in [false, true] {
            let package = compile(&format!(
                "{prefix} fn inspect(pair:*Pair,flag:bool)->bool {{{body}}} fn run()->bool {{let left=true;let right=true;let pair=Pair {{left:left.ref,right:right.ref}};inspect(pair.ref,{flag})}}"
            ));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100000
                )
                .unwrap(),
                Value::Bool(true),
                "{body}, {flag}"
            );
        }
    }
}

#[test]
fn memory_borrowed_pointer_contexts_keep_types_and_actual_aliases() {
    use comptime_interpretor::Value;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);} struct Pair {left:*bool,right:*bool} struct Small {value:*.u8'builtin}";
    let package = compile(&format!(
        "{prefix} fn inspect(pair:*Pair,small:*Small)->bool {{let read_bool:fn(*Pair)->bool=|p|p.left;let read_byte:fn(*Small)->.u8'builtin=|p|p.value;read_bool(pair) and read_byte(small)==7}} fn run()->bool {{let flag=true;let byte:.u8'builtin=7;let pair=Pair {{left:flag.ref,right:flag.ref}};let small=Small {{value:byte.ref}};inspect(pair.ref,small.ref)}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap(),
        Value::Bool(true)
    );
    for flag in [false, true] {
        let package = compile(&format!(
            "{prefix} fn inspect(pair:*Pair,flag:bool)->bool {{if flag {{pair.left.*.* = true;}} else {{pair.right.*.* = true;}}pair.right}} fn run()->bool {{let value=false;let pair=Pair {{left:value.ref,right:value.ref}};inspect(pair.ref,{flag})}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn memory_borrowed_pointer_premises_do_not_replace_caller_validity_checks() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} struct Item {value:bool} struct Pair {left:*bool,right:*bool} fn consume(value:Item) {} fn inspect(pair:*Pair,flag:bool)->bool {if flag {pair.left} else {pair.right}} fn run()->bool {let left=Item {value:true};let right=Item {value:true};let pair=Pair {left:left.value.ref,right:right.value.ref};consume(left);inspect(pair.ref,true)}";
    let package = compile(source);
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_local_closure_calls_preserve_accesses_and_rebinding() {
    use elaboration::verification::memory;
    for body in [
        "let read:fn(*bool)->bool=|p|p.*;read(pointer)",
        "let read:fn(*bool)->bool=|p|p.*;let alias=read;alias(pointer)",
        "var read:fn(*bool)->bool=|p|false;read=|p|p.*;read(pointer)",
        "let read:fn(*bool)->bool=|p|p.*;var alias=read;if flag {alias=read;}alias(pointer)",
    ] {
        let package = compile(&format!(
            "fn inspect(pointer:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value),
            "{body}: {summary}"
        );
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 0).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
        );
    }
    let package = compile(
        "fn inspect(pointer:*bool)->*bool {let forward:fn(*bool)->*bool=|p|p;forward(pointer)}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    let targets = summary.result_targets.unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets.iter().next().unwrap().parameter, 0);
}

#[test]
fn memory_local_closure_identity_cannot_survive_unknown_assignment() {
    use elaboration::verification::memory;
    for body in [
        "read=other;read(pointer)",
        "let selected:fn(*bool)->bool=if flag {read} else {other};selected(pointer)",
        "let selected:fn(*bool)->bool=flag match {true=>read,false=>other};selected(pointer)",
        "if flag {read=other;}read(pointer)",
        "while flag {read(pointer);read=other;}false",
    ] {
        let package = compile(&format!(
            "fn inspect(pointer:*bool,other:fn(*bool)->bool,flag:bool)->bool {{var read:fn(*bool)->bool=|p|false;{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let error =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap_err();
        assert!(error.contains("memory contract"), "{body}: {error}");
    }
}

#[test]
fn memory_local_closure_recursive_edges_use_the_package_fixed_point() {
    use elaboration::verification::memory;
    let package = compile(
        "fn inspect(pointer:*bool,flag:bool)->bool {
        let step:fn(*bool,bool)->bool=|p,again| {if again {inspect(p,false)} else {p.*}};
        step(pointer,flag)
    }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 0 && access.read_value),
        "{summary}"
    );
    assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 10).is_err());
    assert_eq!(
        summary,
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
    );
    let package = compile(
        "fn inspect()->bool {
        let unused:fn(fn(*bool)->bool,*bool)->bool=|callback,p|callback(p);
        true
    }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(summary.accesses.is_empty());
}

#[test]
fn memory_local_closure_parameters_specialize_with_the_enclosing_function() {
    use elaboration::verification::memory;
    let package = compile(
        "fn inspect(pointer:*T)->*T where T {let forward:fn(*T)->*T=|p|p;forward(pointer)}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    for ty in [nir::Primitive::Bool, nir::Primitive::U8] {
        let summary = memory::infer_in_package(
            &package,
            function.declaration,
            &[nir::Term::Primitive(ty)],
            &mut 100000,
        )
        .unwrap();
        assert_eq!(
            summary
                .result_targets
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0
        );
    }
}

#[test]
fn memory_local_closure_candidates_merge_all_branch_and_loop_accesses() {
    use elaboration::verification::memory;
    for body in [
        "let read:fn(*bool,*bool)->bool=if flag {|a,b|a.*} else {|a,b|b.*};read(left,right)",
        "let read:fn(*bool,*bool)->bool=flag match {true=>|a,b|a.*,false=>|a,b|b.*};read(left,right)",
        "var read:fn(*bool,*bool)->bool=|a,b|a.*;if flag {read=|a,b|b.*;}read(left,right)",
        "var read:fn(*bool,*bool)->bool=|a,b|a.*;var again=flag;while again {read(left,right);read=|a,b|b.*;again=false;}read(left,right)",
        "let read:fn(*bool,*bool)->bool=do {let pick:fn(*bool,*bool)->bool=if flag {|a,b|a.*} else {|a,b|b.*};pick};read(left,right)",
    ] {
        let package = compile(&format!(
            "fn inspect(left:*bool,right:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            reads,
            std::collections::BTreeSet::from([0, 1]),
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_local_closure_candidates_preserve_returned_targets_and_runtime_choice() {
    use elaboration::verification::memory;
    for flag in [false, true] {
        let package = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            fn inspect(left:*bool,right:*bool,flag:bool)->*bool {{
                let choose:fn(*bool,*bool)->*bool=if flag {{|a,b|a}} else {{|a,b|b}};
                choose(left,right)
            }}
            fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,{flag}).*}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let targets = summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|target| target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(targets, std::collections::BTreeSet::from([0, 1]));
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 0).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
    }
}

#[test]
fn memory_local_closure_candidates_require_every_actual_target_to_be_live() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn inspect(left:*bool,right:*bool,flag:bool)->bool {
            let read:fn(*bool,*bool)->bool=if flag {|a,b|a.*} else {|a,b|b.*};
            read(left,right)
        }
        fn run()->bool {
            let left=Cell {value:true};let right=Cell {value:false};
            let pointer=right.value.ref;consume(right);
            inspect(left.value.ref,pointer,false)
        }",
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_mixed_named_and_closure_candidates_retain_every_access() {
    use elaboration::verification::memory;
    for body in [
        "let read:fn(*bool,*bool)->bool=if flag {|a,b|a.*} else {named};read(left,right)",
        "let read:fn(*bool,*bool)->bool=flag match {true=>named,false=>|a,b|a.*};read(left,right)",
        "var read:fn(*bool,*bool)->bool=named;if flag {read=|a,b|a.*;}read(left,right)",
        "var read:fn(*bool,*bool)->bool=|a,b|a.*;if flag {read=named;}read(left,right)",
        "var read:fn(*bool,*bool)->bool=named;var again=flag;while again {read(left,right);read=|a,b|a.*;again=false;}read(left,right)",
    ] {
        let package = compile(&format!(
            "fn named(left:*bool,right:*bool)->bool {{right.*}} fn inspect(left:*bool,right:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            reads,
            std::collections::BTreeSet::from([0, 1]),
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_mixed_code_candidates_preserve_generic_instances_and_runtime_choice() {
    use elaboration::verification::memory;
    for flag in [false, true] {
        let package = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            fn named(left:*T,right:*T)->*T where T {{right}}
            fn inspect(left:*bool,right:*bool,flag:bool)->*bool {{
                let choose:fn(*bool,*bool)->*bool=if flag {{|a,b|a}} else {{named<bool>}};
                choose(left,right)
            }}
            fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,{flag}).*}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let targets = summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|target| target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(targets, std::collections::BTreeSet::from([0, 1]));
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 0).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
    }
    let package = compile(
        "fn forward(pointer:*T)->*T where T {pointer}
        fn inspect(flag:*bool,byte:*.u8'builtin,select:bool)->bool {
            let first:fn(*bool)->*bool=if select {|p|p} else {forward<bool>};
            let second:fn(*.u8'builtin)->*.u8'builtin=if select {forward<.u8'builtin>} else {|p|p};
            first(flag).* and second(byte).* == 7
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(summary.accesses.iter().any(|access| access.read_value
        && access.target.parameter == 0
        && access.ty == nir::Term::Primitive(nir::Primitive::Bool)));
    assert!(summary.accesses.iter().any(|access| access.read_value
        && access.target.parameter == 1
        && access.ty == nir::Term::Primitive(nir::Primitive::U8)));
}

#[test]
fn memory_mixed_code_candidates_do_not_hide_unknown_or_dead_targets() {
    use elaboration::verification::memory;
    let package = compile(
        "fn named(p:*bool)->bool {p.*}
        fn inspect(p:*bool,other:fn(*bool)->bool,a:bool,b:bool)->bool {
            let read:fn(*bool)->bool=if a {|p|p.*} else {if b {named} else {other}};
            read(p)
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    assert!(
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000)
            .unwrap_err()
            .contains("memory contract")
    );
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn named(left:*bool,right:*bool)->bool {right.*}
        fn inspect(left:*bool,right:*bool,flag:bool)->bool {
            let read:fn(*bool,*bool)->bool=if flag {|a,b|a.*} else {named};read(left,right)
        }
        fn run()->bool {
            let left=Cell {value:true};let right=Cell {value:false};
            let pointer=right.value.ref;consume(right);inspect(left.value.ref,pointer,false)
        }",
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_local_captured_closures_snapshot_values_and_named_code() {
    use elaboration::verification::memory;
    for body in [
        "let flag=true;let read=|p:*bool| {if flag {p.*} else {false}};read(pointer)",
        "var flag=true;let read=|p:*bool| {if flag {p.*} else {false}};flag=false;read(pointer)",
        "var callback:fn(*bool)->bool=observe;let read=|p:*bool|callback(p);callback=ignore;read(pointer)",
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            fn observe(p:*bool)->bool {{p.*}} fn ignore(p:*bool)->bool {{false}}
            fn inspect(pointer:*bool)->bool {{{body}}}
            fn run()->bool {{let value=true;inspect(value.ref)}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value),
            "{body}: {summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn memory_local_capture_snapshots_include_fields_and_loop_versions() {
    use elaboration::verification::memory;
    let prefix =
        "fn first(a:*bool,b:*bool)->bool {a.*} fn second(a:*bool,b:*bool)->bool {b.* = true;b.*}";
    let package = compile(&format!(
        "{prefix}
        fn inspect(left:*bool,right:*bool,flag:bool)->bool {{
            var callback:fn(*bool,*bool)->bool=first;
            while flag {{let read=|a:*bool,b:*bool|callback(a,b);read(left,right);callback=second;}}
            false
        }}"
    ));
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 0 && access.read_value)
    );
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 1 && access.write)
    );
    assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 10).is_err());
    assert_eq!(
        summary,
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
    );
    let package = compile(&format!(
        "{prefix} struct Ops {{callback:fn(*bool,*bool)->bool}}
        fn inspect(left:*bool,right:*bool)->bool {{
            var ops=Ops {{callback:first}};let read=|a:*bool,b:*bool|ops.callback(a,b);
            ops=Ops {{callback:second}};read(left,right)
        }}"
    ));
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 0 && access.read_value)
    );
    assert!(
        summary
            .accesses
            .iter()
            .all(|access| access.target.parameter != 1)
    );
}

#[test]
fn memory_local_capture_snapshots_never_borrow_later_code_identity() {
    use elaboration::verification::memory;
    for (initial, later, accepted) in [("observe", "input", true), ("input", "observe", false)] {
        let package = compile(&format!(
            "fn observe(p:*bool)->bool {{p.*}}
            fn inspect(pointer:*bool,input:fn(*bool)->bool)->bool {{
                var callback:fn(*bool)->bool={initial};let read=|p:*bool|callback(p);
                callback={later};read(pointer)
            }}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let result = memory::infer_in_package(&package, function.declaration, &[], &mut 100000);
        if accepted {
            assert!(
                result
                    .unwrap()
                    .accesses
                    .iter()
                    .any(|access| access.target.parameter == 0 && access.read_value)
            );
        } else {
            assert!(result.unwrap_err().contains("memory contract"));
        }
    }
    let package = compile("fn inspect(pointer:*bool)->bool {let read=||pointer.*;read()}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert_eq!(summary.accesses.len(), 1);
    assert!(summary.accesses[0].read_value && !summary.accesses[0].write);
    assert_eq!(summary.accesses[0].target.parameter, 0);
}

#[test]
fn memory_local_capture_code_mutation_closes_over_future_invocations() {
    use elaboration::verification::memory;
    for body in [
        "var callback:fn(*bool,*bool)->bool=first;let read=|a:*bool,b:*bool| {let result=callback(a,b);callback=second;result};read(left,right);read(left,right)",
        "var ops=Ops {callback:first};let read=|a:*bool,b:*bool| {let result=ops.callback(a,b);ops.callback=second;result};read(left,right);read(left,right)",
        "var ops=Ops {callback:first};let read=|a:*bool,b:*bool| {let result=ops.callback(a,b);ops=Ops {callback:second};result};read(left,right);read(left,right)",
        "var initial=true;var callback:fn(*bool,*bool)->bool=first;let read=|a:*bool,b:*bool| {if initial {initial=false;callback=second;return true;}callback(a,b)};read(left,right);read(left,right)",
        "var callback:fn(*bool,*bool)->bool=first;let read=|a:*bool,b:*bool| {var again=true;while again {callback=second;again=false;}callback(a,b)};read(left,right);read(left,right)",
    ] {
        let package = compile(&format!(
            "struct Ops {{callback:fn(*bool,*bool)->bool}}
            fn first(a:*bool,b:*bool)->bool {{a.*}} fn second(a:*bool,b:*bool)->bool {{b.*}}
            fn inspect(left:*bool,right:*bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            reads,
            std::collections::BTreeSet::from([0, 1]),
            "{body}: {summary}"
        );
    }
}

#[test]
fn memory_capture_code_invariants_cover_runtime_calls_and_reject_dead_targets() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn first(a:*bool,b:*bool)->bool {a.*} fn second(a:*bool,b:*bool)->bool {b.*}
        fn inspect(left:*bool,right:*bool)->bool {
            var callback:fn(*bool,*bool)->bool=first;
            let read=|a:*bool,b:*bool| {let result=callback(a,b);callback=second;result};
            read(left,right);read(left,right)
        }";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref)}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 10).is_err());
    assert_eq!(
        summary,
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
    );
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let left=Cell {{value:true}};let right=Cell {{value:false}};let pointer=right.value.ref;consume(right);inspect(left.value.ref,pointer)}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_capture_code_invariants_do_not_freeze_invocation_arguments() {
    use elaboration::verification::memory;
    let package=compile("fn first(a:*bool,b:*bool)->bool {a.*} fn second(a:*bool,b:*bool)->bool {b.*}
        fn inspect(left:*bool,right:*bool)->bool {
            var callback:fn(*bool,*bool)->bool=first;
            let read=|next:fn(*bool,*bool)->bool,a:*bool,b:*bool| {let result=callback(a,b);callback=next;result};
            read(second,left,right);read(first,left,right)
        }");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    let roots = summary
        .accesses
        .iter()
        .filter(|access| access.read_value)
        .map(|access| access.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(roots, [0, 1].into_iter().collect(), "{summary}");
}

#[test]
fn memory_local_anonymous_code_fields_follow_projection_assignment_and_patterns() {
    use elaboration::verification::memory;
    for (body, expected) in [
        (
            "let ops=Ops {callback:|a,b|a.*};ops.callback(left,right)",
            vec![0],
        ),
        ("Ops {callback:|a,b|a.*}.callback(left,right)", vec![0]),
        (
            "let ops=flag match {true=>Ops {callback:|a,b|a.*},false=>Ops {callback:|a,b|b.*}};ops.callback(left,right)",
            vec![0, 1],
        ),
        (
            "var ops=Ops {callback:|a,b|a.*};while flag {ops.callback(left,right);ops=Ops {callback:|a,b|b.*};}false",
            vec![0, 1],
        ),
        (
            "let packet=(Ops {callback:|a,b|a.*},|a:*bool,b:*bool|b.*);let (ops,read)=packet;ops.callback(left,right) and read(left,right)",
            vec![0, 1],
        ),
        (
            "let ops=if flag {Ops {callback:|a,b|a.*}} else {Ops {callback:|a,b|b.*}};ops.callback(left,right)",
            vec![0, 1],
        ),
        (
            "var ops=Ops {callback:|a,b|a.*};if flag {ops.callback=|a,b|b.*;}ops.callback(left,right)",
            vec![0, 1],
        ),
        (
            "var ops=Ops {callback:|a,b|a.*};let packet=(ops.callback,do {ops.callback=|a,b|b.*;true});let (read,_)=packet;read(left,right)",
            vec![0],
        ),
    ] {
        let package = compile(&format!(
            "struct Ops {{callback:fn(*bool,*bool)->bool}} fn inspect(left:*bool,right:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, expected.into_iter().collect(), "{body}: {summary}");
    }
}

#[test]
fn memory_local_code_field_writes_invalidate_only_the_written_subtree() {
    use elaboration::verification::memory;
    let prefix = "struct Ops {first:fn(*bool,*bool)->bool,second:fn(*bool,*bool)->bool}";
    for (body, accepted) in [
        ("ops.first=other;ops.second(left,right)", true),
        ("ops.first=other;ops.first(left,right)", false),
        ("if flag {ops.first=other;}ops.first(left,right)", false),
        (
            "while flag {ops.first(left,right);ops.first=other;}false",
            false,
        ),
        (
            "let chosen=if flag {ops} else {Ops {first:other,second:other}};chosen.first(left,right)",
            false,
        ),
    ] {
        let package=compile(&format!("{prefix} fn inspect(left:*bool,right:*bool,other:fn(*bool,*bool)->bool,flag:bool)->bool {{
            var ops=Ops {{first:|a,b|a.*,second:|a,b|b.*}};{body}
        }}"));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let result = memory::infer_in_package(&package, function.declaration, &[], &mut 100000);
        if accepted {
            let summary = result.unwrap();
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|access| access.read_value && access.target.parameter == 1)
            );
            assert!(
                summary
                    .accesses
                    .iter()
                    .all(|access| access.target.parameter != 0)
            );
        } else {
            assert!(result.unwrap_err().contains("memory contract"), "{body}");
        }
    }
}

#[test]
fn memory_local_anonymous_field_calls_execute_and_check_actual_targets() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        struct Ops {callback:fn(*bool)->bool}
        fn inspect(pointer:*bool)->bool {let ops=Ops {callback:|p|p.*};ops.callback(pointer)}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let value=true;inspect(value.ref)}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 10).is_err());
    assert_eq!(
        summary,
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
    );
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let cell=Cell {{value:true}};let pointer=cell.value.ref;consume(cell);inspect(pointer)}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_local_higher_order_calls_transport_anonymous_argument_evidence() {
    use elaboration::verification::memory;
    for body in [
        "let invoke=|callback:fn(*bool)->bool,p:*bool|callback(p);invoke(|p|p.*,pointer)",
        "let invoke=|ops:Ops,p:*bool|ops.callback(p);invoke(Ops {callback:|p|p.*},pointer)",
        "let invoke=|callback:%{flag:bool} fn(*bool)->bool,p:*bool|callback(p);invoke(|p:*bool|flag and p.*,pointer)",
        "let invoke=|callback:fn(*bool)->bool,p:*bool| {let forward=|action:fn(*bool)->bool,target:*bool|action(target);forward(callback,p)};invoke(|p|p.*,pointer)",
        "let invoke=|callback:fn(*bool)->bool,p:*bool|callback(p);let callback:fn(*bool)->bool=if flag {|p|p.*} else {named};invoke(callback,pointer)",
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ops {{callback:fn(*bool)->bool}} fn named(p:*bool)->bool {{p.*}}
            fn inspect(pointer:*bool,flag:bool)->bool {{{body}}}
            fn run()->bool {{let value=true;inspect(value.ref,true)}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value),
            "{body}: {summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn memory_local_higher_order_arguments_snapshot_before_later_evaluation() {
    use elaboration::verification::memory;
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        fn inspect(left:*bool,right:*bool)->bool {
            let invoke=|callback:fn(*bool,*bool)->bool,a:*bool,b:*bool|callback(a,b);
            var callback:fn(*bool,*bool)->bool=|a,b|a.*;
            invoke(callback,do {callback=|a,b|b.*;left},right)
        }
        fn run()->bool {let left=true;let right=false;inspect(left.ref,right.ref)}";
    let package = compile(source);
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    let reads = summary
        .accesses
        .iter()
        .filter(|access| access.read_value)
        .map(|access| access.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, std::collections::BTreeSet::from([0]));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 10).is_err());
    assert_eq!(
        summary,
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap()
    );
}

#[test]
fn memory_local_higher_order_arguments_preserve_unknown_and_dead_targets() {
    use elaboration::verification::memory;
    for body in [
        "let callback:fn(*bool)->bool=if flag {|p|p.*} else {unknown};invoke(callback,pointer)",
        "var callback:fn(*bool)->bool=|p|p.*;if flag {callback=unknown;}invoke(callback,pointer)",
    ] {
        let package = compile(&format!(
            "fn inspect(pointer:*bool,unknown:fn(*bool)->bool,flag:bool)->bool {{let invoke=|callback:fn(*bool)->bool,p:*bool|callback(p);{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        assert!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000)
                .unwrap_err()
                .contains("memory contract"),
            "{body}"
        );
    }
    for invocation in [
        "callback(p)",
        "{let forward=|action:fn(*bool)->bool,target:*bool|action(target);forward(callback,p)}",
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Cell {{value:bool}} fn consume(value:Cell) {{}}
            fn inspect(pointer:*bool)->bool {{let invoke=|callback:fn(*bool)->bool,p:*bool|{invocation};invoke(|p|p.*,pointer)}}
            fn run()->bool {{let cell=Cell {{value:true}};let pointer=cell.value.ref;consume(cell);inspect(pointer)}}"));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100000,
        )
        .unwrap_err();
        assert!(error.contains("unavailable"), "{error}");
    }
}

#[test]
fn memory_prepared_anonymous_evidence_checks_unused_parameter_layout() {
    use elaboration::verification::{
        memory,
        moves::{self, Delivery},
    };
    use std::collections::BTreeMap;
    let package = compile(
        "fn receiver()->fn(fn(*bool)->bool,*bool)->bool {|callback:fn(*bool)->bool,p:*bool|true}
        fn argument()->fn(*bool)->bool {|p:*bool|p.*}
        fn captured()->%{flag:bool} fn(*bool)->bool {let flag=true;|p:*bool|flag and p.*}",
    );
    let evidence = |name: &str| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        moves::analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 100000)
            .unwrap()
            .callable_evidence_exits[0]
            .as_ref()
            .unwrap()
            .definition
            .clone()
    };
    let receiver = evidence("receiver");
    let argument = evidence("argument");
    let input = memory::CallableInput {
        parameter: 0,
        fields: vec![],
    };
    let captures = std::sync::Arc::new(BTreeMap::new());
    let infer = |definition: &nir::program::Expr| {
        let code = BTreeMap::from([(
            input.clone(),
            vec![memory::CodeDefinition::Closure {
                definition,
                captures: captures.clone(),
                prepared_captures: Default::default(),
            }],
        )]);
        memory::infer_closure_with_code_inputs(
            &package,
            &receiver,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &code,
            &mut 100000,
        )
    };
    assert!(infer(&argument).unwrap().accesses.is_empty());
    let mut wrong = evidence("captured");
    wrong.ty = argument.ty.clone();
    assert!(infer(&wrong).unwrap_err().contains("layout disagrees"));
    let mut wrong = argument.clone();
    if let nir::program::ExprKind::Closure { parameters, .. } = &mut wrong.kind {
        parameters.clear();
    }
    assert!(infer(&wrong).unwrap_err().contains("layout disagrees"));
    let empty = BTreeMap::from([(input.clone(), vec![])]);
    assert!(
        memory::infer_closure_with_code_inputs(
            &package,
            &receiver,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &empty,
            &mut 100000
        )
        .unwrap_err()
        .contains("empty prepared")
    );
    let code = BTreeMap::from([(
        input,
        vec![memory::CodeDefinition::Closure {
            definition: &argument,
            captures,
            prepared_captures: Default::default(),
        }],
    )]);
    assert!(
        memory::infer_closure_with_code_inputs(
            &package,
            &receiver,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &code,
            &mut 0
        )
        .unwrap_err()
        .contains("budget")
    );
    assert!(
        memory::infer_closure_with_code_inputs(
            &package,
            &receiver,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &code,
            &mut 100000
        )
        .unwrap()
        .accesses
        .is_empty()
    );
}

#[test]
fn memory_named_higher_order_calls_keep_anonymous_argument_candidates() {
    use elaboration::verification::memory;
    for body in [
        "invoke(|a,b|a.*,left,right)",
        "let callback:fn(*bool,*bool)->bool=if flag {|a,b|a.*} else {|a,b|b.*};invoke(callback,left,right)",
        "let first=invoke(|a,b|a.*,left,right);first and invoke(|a,b|b.*,left,right)",
    ] {
        let package = compile(&format!(
            "fn invoke(callback:fn(*bool,*bool)->bool,a:*bool,b:*bool)->bool {{callback(a,b)}} fn inspect(left:*bool,right:*bool,flag:bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            reads,
            if body.starts_with("let") {
                std::collections::BTreeSet::from([0, 1])
            } else {
                std::collections::BTreeSet::from([0])
            }
        );
    }
}

#[test]
fn memory_named_anonymous_callbacks_execute_across_recursion_and_specialization() {
    use elaboration::verification::memory;
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ops {callback:fn(*bool)->bool}
        fn invoke(callback:fn(*T)->T,p:*T)->T where T {callback(p)}
        fn recurse(callback:fn(*bool)->bool,p:*bool,again:bool)->bool {if again {recurse(callback,p,false)} else {callback(p)}}
        fn record(ops:Ops,p:*bool)->bool {ops.callback(p)}
        fn captured(callback:%{flag:bool} fn(*bool)->bool,p:*bool)->bool {callback(p)}";
    for body in [
        "invoke(|p|p.*,pointer)",
        "recurse(|p|p.*,pointer,true)",
        "record(Ops {callback:|p|p.*},pointer)",
        "captured(|p:*bool|flag and p.*,pointer)",
        "let apply=|callback:fn(*bool)->bool,p:*bool|invoke(callback,p);apply(|p|p.*,pointer)",
    ] {
        let package = compile(&format!(
            "{prefix} fn inspect(pointer:*bool,flag:bool)->bool {{{body}}} fn run()->bool {{let value=true;inspect(value.ref,true)}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary = memory::infer_in_package(&package, function.declaration, &[], &mut 200000)
            .unwrap_or_else(|e| panic!("{body}: {e}"));
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value),
            "{summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 20).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, function.declaration, &[], &mut 200000).unwrap()
        );
    }
}

#[test]
fn memory_named_anonymous_arguments_retain_snapshot_unknown_and_liveness_checks() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn invoke(callback:fn(*bool,*bool)->bool,a:*bool,b:*bool)->bool {callback(a,b)}";
    let package=compile(&format!("{prefix}
        fn inspect(a:*bool,b:*bool)->bool {{var callback:fn(*bool,*bool)->bool=|a,b|a.*;invoke(callback,do {{callback=|a,b|b.*;a}},b)}}
        fn run()->bool {{let a=true;let b=false;inspect(a.ref,b.ref)}}"));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 0)
    );
    assert!(
        !summary
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 1)
    );
    let package = compile(&format!(
        "{prefix} fn inspect(a:*bool,b:*bool,unknown:fn(*bool,*bool)->bool,flag:bool)->bool {{let callback:fn(*bool,*bool)->bool=if flag {{|a,b|a.*}} else {{unknown}};invoke(callback,a,b)}}"
    ));
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    assert!(
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 100000)
            .unwrap_err()
            .contains("memory contract")
    );
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let cell=Cell {{value:true}};let pointer=cell.value.ref;consume(cell);invoke(|a,b|b.*,pointer,pointer)}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_named_prepared_evidence_is_retained_and_validated() {
    use elaboration::verification::{
        memory,
        moves::{self, Delivery},
    };
    use std::collections::BTreeMap;
    let package = compile(
        "fn invoke(callback:fn(*bool,*bool)->bool,a:*bool,b:*bool)->bool {callback(a,b)}
        fn first()->fn(*bool,*bool)->bool {|a:*bool,b:*bool|a.*}
        fn second()->fn(*bool,*bool)->bool {|a:*bool,b:*bool|b.*}",
    );
    let evidence = |name: &str| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        moves::analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 100000)
            .unwrap()
            .callable_evidence_exits[0]
            .as_ref()
            .unwrap()
            .definition
            .clone()
    };
    let first = evidence("first");
    let second = evidence("second");
    let invoke = package
        .functions
        .iter()
        .find(|f| f.name == "invoke")
        .unwrap();
    let input = memory::CallableInput {
        parameter: 0,
        fields: vec![],
    };
    let make = |definition| {
        BTreeMap::from([(
            input.clone(),
            vec![memory::CodeDefinition::Closure {
                definition,
                captures: Default::default(),
                prepared_captures: Default::default(),
            }],
        )])
    };
    let first_code = make(&first);
    let second_code = make(&second);
    let infer = |code: &memory::CodeInputs<'_>, fuel| {
        memory::infer_with_code_inputs(
            &package,
            invoke.declaration,
            &[],
            &BTreeMap::new(),
            code,
            &mut { fuel },
        )
    };
    let left = infer(&first_code, 100000).unwrap();
    let right = infer(&second_code, 100000).unwrap();
    assert_ne!(left.prepared_inputs, right.prepared_inputs);
    assert!(
        left.accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 1)
    );
    assert!(
        !left
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 2)
    );
    assert!(
        right
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 2)
    );
    assert!(
        left.to_string()
            .contains("(prepared-inputs (premise (callable-input")
    );
    // Result evidence belongs to the summary, outside every input premise.
    // Parsing the nesting here also catches unbalanced or multiple root dumps.
    let dump = left.to_string();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut result_fields = 0;
    for (index, ch) in dump.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        if ch == '"' {
            quoted = true;
            continue;
        }
        if ch == '(' {
            if dump[index..].starts_with("(code-fields") {
                assert_eq!(depth, 1, "{dump}");
                result_fields += 1;
            }
            if index != 0 {
                assert!(depth > 0, "multiple dump roots: {dump}");
            }
            depth += 1;
        } else if ch == ')' {
            depth = depth.checked_sub(1).expect("unbalanced dump");
        }
    }
    assert_eq!(depth, 0, "{dump}");
    assert!(!quoted, "{dump}");
    assert_eq!(result_fields, 1, "{dump}");
    let mut malformed = first.clone();
    if let nir::program::ExprKind::Closure { parameters, .. } = &mut malformed.kind {
        parameters[1] = parameters[0];
    }
    assert!(
        infer(&make(&malformed), 100000)
            .unwrap_err()
            .contains("binding collision")
    );
    let missing = BTreeMap::from([(
        memory::CallableInput {
            parameter: 8,
            fields: vec![],
        },
        first_code[&input].clone(),
    )]);
    assert!(
        infer(&missing, 100000)
            .unwrap_err()
            .contains("parameter is missing")
    );
    assert!(infer(&first_code, 0).unwrap_err().contains("budget"));
    assert_eq!(left, infer(&first_code, 100000).unwrap());
}

#[test]
fn memory_named_anonymous_code_specializes_in_the_defining_frame() {
    use elaboration::verification::memory;
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Byte=.u8'builtin;
        fn invoke(callback:fn(*T)->*T,p:*T)->*T where T {callback(p)}
        fn inspect(p:*T)->*T where T {invoke(|p|p,p)}
        fn run()->bool {let flag=true;let byte:Byte=7;inspect(flag.ref).* and inspect(byte.ref).* == 7}");
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    for ty in [
        nir::Term::Primitive(nir::Primitive::Bool),
        nir::Term::Primitive(nir::Primitive::U8),
    ] {
        let summary = memory::infer_in_package(
            &package,
            inspect.declaration,
            std::slice::from_ref(&ty),
            &mut 100000,
        )
        .unwrap();
        let nir::Term::Function {
            parameters, result, ..
        } = &summary.signature
        else {
            panic!("missing signature")
        };
        assert_eq!(parameters[0].ty, nir::Term::Pointer(Box::new(ty.clone())));
        assert_eq!(**result, nir::Term::Pointer(Box::new(ty)));
        assert_eq!(
            summary
                .result_targets
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0
        );
    }
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn memory_named_callback_captures_keep_construction_snapshots() {
    use elaboration::verification::memory;
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        fn first(a:*bool,b:*bool)->bool {a.*}
        fn second(a:*bool,b:*bool)->bool {b.*}
        fn invoke(callback:%{action:fn(*bool,*bool)->bool} fn(*bool,*bool)->bool,a:*bool,b:*bool)->bool {callback(a,b)}";
    for (body, expected) in [
        (
            "var action=first;let callback=|a:*bool,b:*bool|action(a,b);action=second;invoke(callback,left,right)",
            vec![0],
        ),
        (
            "var action=first;var again=true;while again {let callback=|a:*bool,b:*bool|action(a,b);invoke(callback,left,right);action=second;again=flag;}true",
            vec![0, 1],
        ),
    ] {
        let package = compile(&format!(
            "{prefix} fn inspect(left:*bool,right:*bool,flag:bool)->bool {{{body}}} fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,false)}}"
        ));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary = memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000)
            .unwrap_or_else(|e| panic!("{body}: {e}"));
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, expected.into_iter().collect(), "{summary}");
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn memory_named_callback_state_transitions_accept_match_defined_code() {
    use elaboration::verification::memory;
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        fn first(a:*bool,b:*bool)->bool {a.*}
        fn second(a:*bool,b:*bool)->bool {b.*}
        fn invoke(callback:%{action:fn(*bool,*bool)->bool} fn(*bool,*bool,bool)->bool,a:*bool,b:*bool,flag:bool)->bool {callback(a,b,flag);callback(a,b,flag)}";
    let package=compile(&format!("{prefix}
        fn inspect(left:*bool,right:*bool,flag:bool)->bool {{var action=first;let callback=|a:*bool,b:*bool,choose:bool| {{let result=action(a,b);action=choose match {{true=>first,false=>second}};result}};invoke(callback,left,right,flag)}}
        fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,false)}}"));
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000).unwrap();
    let reads = summary
        .accesses
        .iter()
        .filter(|a| a.read_value)
        .map(|a| a.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, std::collections::BTreeSet::from([0, 1]));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 400000)
            .unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
    assert!(memory::infer_in_package(&package, inspect.declaration, &[], &mut 20).is_err());
    assert_eq!(
        summary,
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000).unwrap()
    );
}

#[test]
fn memory_match_capture_updates_do_not_freeze_invocation_inputs() {
    use elaboration::verification::memory;
    let package = compile(
        "fn first(a:*bool,b:*bool)->bool {a.*}
        fn inspect(left:*bool,right:*bool,flag:bool)->bool {
            var action=first;
            let callback=|a:*bool,b:*bool,next:fn(*bool,*bool)->bool,choose:bool| {
                let result=action(a,b);action=choose match {true=>first,false=>next};result
            };
            callback(left,right,first,flag)
        }",
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 100000).unwrap();
    let roots = summary
        .accesses
        .iter()
        .filter(|access| access.read_value)
        .map(|access| access.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(roots, [0].into_iter().collect(), "{summary}");
}

#[test]
fn memory_match_capture_updates_cover_owned_fields() {
    use elaboration::verification::memory;
    for update in [
        "ops.callback=flag match {true=>first,false=>second}",
        "ops=flag match {true=>Ops {callback:first},false=>Ops {callback:second}}",
    ] {
        let package = compile(&format!(
            "struct Ops {{callback:fn(*bool,*bool)->bool}}
            fn first(a:*bool,b:*bool)->bool {{a.*}} fn second(a:*bool,b:*bool)->bool {{b.*}}
            fn inspect(left:*bool,right:*bool,flag:bool)->bool {{
                var ops=Ops {{callback:first}};
                let callback=|a:*bool,b:*bool| {{let result=ops.callback(a,b);{update};result}};
                callback(left,right);callback(left,right)
            }}"
        ));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 100000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            reads,
            std::collections::BTreeSet::from([0, 1]),
            "{update}: {summary}"
        );
    }
}

#[test]
fn memory_capture_code_can_be_established_before_its_first_use() {
    use elaboration::verification::memory;
    let prefix="fn read(p:*bool)->bool {p.*}
        fn invoke(callback:%{action:fn(*bool)->bool} fn(*bool,bool)->bool,p:*bool,flag:bool)->bool {callback(p,flag)}";
    for (body, accepted) in [
        ("action=read;action(p)", true),
        ("action=|p|p.*;action(p)", true),
        ("if choose {action=|p|p.*;}action(p)", false),
        (
            "action=choose match {true=>read,false=>read};action(p)",
            true,
        ),
        ("if choose {action=read;}action(p)", false),
        ("let result=action(p);action=read;result", false),
    ] {
        for call in ["callback(pointer,flag)", "invoke(callback,pointer,flag)"] {
            let package = compile(&format!(
                "{prefix} fn inspect(pointer:*bool,input:fn(*bool)->bool,flag:bool)->bool {{
                var action=input;let callback=|p:*bool,choose:bool| {{{body}}};{call}
            }}"
            ));
            let inspect = package
                .functions
                .iter()
                .find(|f| f.name == "inspect")
                .unwrap();
            let result = memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000);
            if accepted {
                let summary = result.unwrap_or_else(|e| panic!("{body}; {call}: {e}"));
                assert!(
                    summary
                        .accesses
                        .iter()
                        .any(|a| a.read_value && a.target.parameter == 0)
                );
                assert!(
                    memory::infer_in_package(&package, inspect.declaration, &[], &mut 10).is_err()
                );
                assert_eq!(
                    summary,
                    memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000)
                        .unwrap()
                );
            } else {
                assert!(
                    result.unwrap_err().contains("memory contract"),
                    "{body}; {call}"
                );
            }
        }
    }
}

#[test]
fn memory_established_capture_code_checks_actual_targets() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn read(p:*bool)->bool {p.*} fn ignore(p:*bool)->bool {true}
        fn invoke(callback:%{action:fn(*bool)->bool} fn(*bool)->bool,p:*bool)->bool {callback(p)}
        fn inspect(pointer:*bool,input:fn(*bool)->bool)->bool {
            var action=input;let callback=|p:*bool| {action=read;action(p)};invoke(callback,pointer)
        }";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let value=false;inspect(value.ref,ignore)}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300000)
            .unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let cell=Cell {{value:false}};let pointer=cell.value.ref;consume(cell);inspect(pointer,ignore)}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_anonymous_captured_callbacks_cross_local_and_named_boundaries() {
    use elaboration::verification::memory;
    for call in ["wrapper(pointer)", "invoke(wrapper,pointer)"] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            fn invoke(callback:%{{read:fn(*bool)->bool}} fn(*bool)->bool,p:*bool)->bool {{callback(p)}}
            fn inspect(pointer:*bool)->bool {{let read:fn(*bool)->bool=|p|p.*;let wrapper=|p:*bool|read(p);{call}}}
            fn run()->bool {{let value=true;inspect(value.ref)}}"));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.read_value && a.target.parameter == 0)
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                400000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn memory_anonymous_capture_snapshots_preserve_mixed_candidates_and_fields() {
    use elaboration::verification::memory;
    for (body, expected) in [
        (
            "let read:fn(*bool,*bool)->bool=if flag {|a,b|a.*} else {second};let wrapper=|a:*bool,b:*bool|read(a,b);wrapper(left,right)",
            vec![0, 1],
        ),
        (
            "var read:fn(*bool,*bool)->bool=|a,b|a.*;let wrapper=|a:*bool,b:*bool|read(a,b);read=|a,b|b.*;wrapper(left,right)",
            vec![0],
        ),
        (
            "let ops=Ops {read:|a,b|a.*};let wrapper=|a:*bool,b:*bool|ops.read(a,b);wrapper(left,right)",
            vec![0],
        ),
        (
            "let read:fn(*bool,*bool)->bool=|a,b|a.*;let middle=|a:*bool,b:*bool|read(a,b);let wrapper=|a:*bool,b:*bool|middle(a,b);wrapper(left,right)",
            vec![0],
        ),
        (
            "var action=first;let inner=|a:*bool,b:*bool| {let result=action(a,b);action=second;result};let outer=|a:*bool,b:*bool|inner(a,b);outer(left,right);not outer(left,right)",
            vec![0, 1],
        ),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ops {{read:fn(*bool,*bool)->bool}} fn first(a:*bool,b:*bool)->bool {{a.*}} fn second(a:*bool,b:*bool)->bool {{b.*}}
            fn inspect(left:*bool,right:*bool,flag:bool)->bool {{{body}}}
            fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,true)}}"));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary = memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000)
            .unwrap_or_else(|e| panic!("{body}: {e}"));
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, expected.into_iter().collect(), "{body}: {summary}");
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(memory::infer_in_package(&package, inspect.declaration, &[], &mut 10).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap()
        );
    }
}

#[test]
fn memory_anonymous_captures_never_hide_unknown_or_dead_targets() {
    use elaboration::verification::memory;
    let package=compile("fn inspect(pointer:*bool,input:fn(*bool)->bool,flag:bool)->bool {
        let read:fn(*bool)->bool=if flag {|p|p.*} else {input};let wrapper=|p:*bool|read(p);wrapper(pointer)
    }");
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    assert!(
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 100000)
            .unwrap_err()
            .contains("memory contract")
    );
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn inspect(pointer:*bool)->bool {let read:fn(*bool)->bool=|p|p.*;let wrapper=|p:*bool|read(p);wrapper(pointer)}
        fn run()->bool {let cell=Cell {value:true};let pointer=cell.value.ref;consume(cell);inspect(pointer)}");
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_prepared_capture_evidence_validates_paths_and_environment_identity() {
    use elaboration::verification::{
        memory,
        moves::{self, Delivery},
    };
    use std::collections::BTreeMap;
    let package = compile(
        "fn make(read:fn(*bool)->bool)->%{read:fn(*bool)->bool} fn(*bool)->bool {|p:*bool|read(p)}
        fn leaf()->fn(*bool)->bool {|p:*bool|p.*} fn named(p:*bool)->bool {p.*}",
    );
    let evidence = |name: &str| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        moves::analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 100000)
            .unwrap()
            .callable_evidence_exits[0]
            .as_ref()
            .unwrap()
            .definition
            .clone()
    };
    let wrapper = evidence("make");
    let leaf = evidence("leaf");
    let nir::program::ExprKind::Closure { captures, .. } = &wrapper.kind else {
        panic!("not a closure")
    };
    let place = memory::CallablePlace {
        binding: captures[0].binding,
        fields: vec![],
    };
    let code = memory::PreparedCode::Closure {
        definition: std::sync::Arc::new(leaf),
        captures: Default::default(),
        prepared_captures: Default::default(),
    };
    let inputs = BTreeMap::from([(place.clone(), vec![code.clone()])]);
    let infer = |inputs: &memory::PreparedCaptures, fuel| {
        memory::infer_closure_with_prepared_captures(
            &package,
            &wrapper,
            &BTreeMap::new(),
            inputs,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &mut { fuel },
        )
    };
    let summary = infer(&inputs, 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 0)
    );
    assert!(code.to_string().contains("(prepared-captures"));
    let mut unknown = place.clone();
    unknown.binding.index += 100;
    assert!(
        infer(&BTreeMap::from([(unknown, vec![code.clone()])]), 100000)
            .unwrap_err()
            .contains("unknown capture")
    );
    let mut field = place.clone();
    field.fields.push(0);
    assert!(infer(&BTreeMap::from([(field, vec![code])]), 100000).is_err());
    assert!(
        infer(&BTreeMap::from([(place.clone(), vec![])]), 100000)
            .unwrap_err()
            .contains("empty prepared")
    );
    let named = package
        .functions
        .iter()
        .find(|f| f.name == "named")
        .unwrap();
    let names = BTreeMap::from([(
        place,
        memory::CallableResult::Known(iir::InstanceKey {
            declaration: named.declaration,
            arguments: vec![],
        }),
    )]);
    assert!(
        memory::infer_closure_with_prepared_captures(
            &package,
            &wrapper,
            &names,
            &inputs,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &mut 100000
        )
        .unwrap_err()
        .contains("conflicting")
    );
    assert!(infer(&inputs, 0).unwrap_err().contains("budget"));
    assert_eq!(summary, infer(&inputs, 100000).unwrap());
}

#[test]
fn memory_anonymous_capture_specialization_keeps_nested_definition_frames() {
    use elaboration::verification::memory;
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Byte=.u8'builtin;
        fn invoke(callback:%{read:fn(*T)->*T} fn(*T)->*T,p:*T)->*T where T {callback(p)}
        fn inspect(p:*T)->*T where T {let read:fn(*T)->*T=|p|p;let wrapper=|p:*T|read(p);invoke(wrapper,p)}
        fn run()->bool {let flag=true;let byte:Byte=7;inspect(flag.ref).* and inspect(byte.ref).* == 7}");
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    for ty in [nir::Primitive::Bool, nir::Primitive::U8] {
        let summary = memory::infer_in_package(
            &package,
            inspect.declaration,
            &[nir::Term::Primitive(ty)],
            &mut 300000,
        )
        .unwrap();
        assert_eq!(
            summary
                .result_targets
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0
        );
    }
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn memory_captured_code_transitions_include_fixed_anonymous_definitions() {
    use elaboration::verification::memory;
    for (initial, next) in [
        ("first", "|a,b|b.*"),
        ("|a,b|a.*", "second"),
        ("|a,b|a.*", "|a,b|b.*"),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            fn first(a:*bool,b:*bool)->bool {{a.*}} fn second(a:*bool,b:*bool)->bool {{b.*}}
            fn invoke(callback:%{{action:fn(*bool,*bool)->bool}} fn(*bool,*bool)->bool,a:*bool,b:*bool)->bool {{callback(a,b);callback(a,b)}}
            fn inspect(left:*bool,right:*bool)->bool {{var action:fn(*bool,*bool)->bool={initial};let callback=|a:*bool,b:*bool| {{let result=action(a,b);action={next};result}};invoke(callback,left,right)}}
            fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref)}}"));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            reads,
            std::collections::BTreeSet::from([0, 1]),
            "{initial} => {next}: {summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(false)
        );
    }
}

#[test]
fn memory_fixed_anonymous_capture_updates_cover_fields_and_branch_choices() {
    use elaboration::verification::memory;
    for update in [
        "ops.action=|a,b|b.*",
        "ops=Ops {action:|a,b|b.*}",
        "ops.action=if flag {first} else {|a,b|b.*}",
        "ops=flag match {true=>Ops {action:|a,b|a.*},false=>Ops {action:|a,b|b.*}}",
    ] {
        let package = compile(&format!(
            "struct Ops {{action:fn(*bool,*bool)->bool}}
            fn first(a:*bool,b:*bool)->bool {{a.*}}
            fn inspect(left:*bool,right:*bool,flag:bool)->bool {{var ops=Ops {{action:first}};
                let callback=|a:*bool,b:*bool| {{let result=ops.action(a,b);{update};result}};
                callback(left,right);callback(left,right)
            }}"
        ));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary = memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000)
            .unwrap_or_else(|e| panic!("{update}: {e}"));
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, std::collections::BTreeSet::from([0, 1]), "{summary}");
        assert!(memory::infer_in_package(&package, inspect.declaration, &[], &mut 10).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap()
        );
    }
}

#[test]
fn memory_fixed_anonymous_capture_updates_reject_a_dead_later_target() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn first(a:*bool,b:*bool)->bool {a.*}
        fn run()->bool {let left=true;let right=Cell {value:false};let pointer=right.value.ref;
            var action:fn(*bool,*bool)->bool=first;
            let callback=|a:*bool,b:*bool| {let result=action(a,b);action=|a,b|b.*;result};
            callback(left.ref,pointer);consume(right);callback(left.ref,pointer)
        }",
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 400000)
            .unwrap_err();
    assert!(error.contains("memory call target unavailable"), "{error}");
}

#[test]
fn memory_capture_replacement_does_not_freeze_dynamic_environments() {
    use elaboration::verification::memory;
    let package = compile(
        "fn read(p:*bool)->bool {p.*}
        fn inspect(pointer:*bool,input:fn(*bool)->bool)->bool {
            let selected=read;var action=|p:*bool|selected(p);
            let wrapper=|p:*bool,next:fn(*bool)->bool| {
                let selected=next;action=|p:*bool|selected(p);action(p)
            };
            wrapper(pointer,input)
        }",
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let error =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000).unwrap_err();
    assert!(
        error.contains("dynamic call requires a richer memory contract"),
        "{error}"
    );
}

#[test]
fn memory_capture_record_replacement_keeps_environment_bearing_code_targets() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Ops {action:%{choose:bool} fn(*bool,*bool)->bool}
        fn inspect(left:*bool,right:*bool)->bool {
            let choose=true;var ops=Ops {action:|a:*bool,b:*bool|if choose {a.*} else {a.*}};
            let callback=|a:*bool,b:*bool| {
                let result=ops.action(a,b);
                ops=do {let choose=false;Ops {action:|a:*bool,b:*bool|if choose {b.*} else {b.*}}};
                result
            };
            callback(left,right);callback(left,right)
        }",
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap();
    let reads = summary
        .accesses
        .iter()
        .filter(|a| a.read_value)
        .map(|a| a.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, std::collections::BTreeSet::from([0, 1]), "{summary}");
}

#[test]
fn memory_data_capture_replacement_abstracts_invocation_values() {
    use elaboration::verification::memory;
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        fn invoke(callback:%{action:%{choose:bool} fn(*bool,*bool)->bool} fn(*bool,*bool,bool)->bool,a:*bool,b:*bool,next:bool)->bool {callback(a,b,next);callback(a,b,next)}
        fn inspect(left:*bool,right:*bool,next:bool)->bool {
            let choose=true;var action=|a:*bool,b:*bool|if choose {a.*} else {a.*};
            let wrapper=|a:*bool,b:*bool,next:bool| {
                let result=action(a,b);let choose=next;
                action=|a:*bool,b:*bool|if choose {a.*} else {b.*};result
            };
            invoke(wrapper,left,right,next)
        }";
    for flag in [true, false] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,{flag})}}"
        ));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, std::collections::BTreeSet::from([0, 1]));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                600000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
        assert!(memory::infer_in_package(&package, inspect.declaration, &[], &mut 10).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap()
        );
    }
}

#[test]
fn memory_data_capture_replacement_tracks_nested_plain_records() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Choice {flag:bool} struct Outer {choice:Choice}
        fn inspect(left:*bool,right:*bool,next:bool)->bool {
            let data=Outer {choice:Choice {flag:true}};
            var action=|a:*bool,b:*bool|if data.choice.flag {a.*} else {a.*};
            let wrapper=|a:*bool,b:*bool,next:bool| {
                let result=action(a,b);let data=Outer {choice:Choice {flag:next}};
                action=|a:*bool,b:*bool|if data.choice.flag {a.*} else {b.*};result
            };
            wrapper(left,right,next);wrapper(left,right,next)
        }",
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap();
    let reads = summary
        .accesses
        .iter()
        .filter(|a| a.read_value)
        .map(|a| a.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, std::collections::BTreeSet::from([0, 1]));
}

#[test]
fn memory_record_environment_replacement_rejects_hidden_code_capture() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Payload {code:fn(*bool)->bool}
        struct Ops {action:%{payload:Payload} fn(*bool)->bool}
        fn read(p:*bool)->bool {p.*}
        fn inspect(pointer:*bool,input:fn(*bool)->bool)->bool {
            let payload=Payload {code:read};var ops=Ops {action:|p:*bool|payload.code(p)};
            let wrapper=|p:*bool,next:fn(*bool)->bool| {
                let result=ops.action(p);let payload=Payload {code:next};
                ops=Ops {action:|p:*bool|payload.code(p)};result
            };
            wrapper(pointer,input);wrapper(pointer,input)
        }",
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let error =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000).unwrap_err();
    assert!(
        error.contains("dynamic call requires a richer memory contract"),
        "{error}"
    );
}

#[test]
fn memory_record_environment_replacement_checks_the_later_actual_target() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        struct Ops {action:%{choose:bool} fn(*bool,*bool)->bool}
        fn inspect(left:*bool,right:*bool)->bool {
            let choose=true;var ops=Ops {action:|a:*bool,b:*bool|if choose {a.*} else {a.*}};
            let callback=|a:*bool,b:*bool| {
                let result=ops.action(a,b);
                ops=do {let choose=false;Ops {action:|a:*bool,b:*bool|if choose {b.*} else {b.*}}};result
            };
            callback(left,right);callback(left,right)
        }";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref)}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 600000)
            .unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let left=true;let right=Cell {{value:false}};let pointer=right.value.ref;consume(right);inspect(left.ref,pointer)}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 600000)
            .unwrap_err();
    assert!(error.contains("memory call target unavailable"), "{error}");
}

#[test]
fn memory_returned_anonymous_callbacks_keep_their_pointer_footprints() {
    use elaboration::verification::memory;
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn make()->fn(*bool)->bool {|p:*bool|p.*}
        fn inspect(pointer:*bool)->bool {let callback=make();callback(pointer)}
        fn run()->bool {let value=true;inspect(value.ref)}",
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 200000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 0)
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 400000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn memory_returned_code_keeps_captures_fields_and_recursive_choices() {
    use elaboration::verification::memory;
    for (definitions, body, expected) in [
        (
            "fn make(flag:bool)->%{flag:bool} fn(*bool,*bool)->bool {|a:*bool,b:*bool|if flag {a.*} else {b.*}}",
            "let callback=make(flag);callback(left,right)",
            vec![0, 1],
        ),
        (
            "struct Ops {action:fn(*bool,*bool)->bool} fn make(flag:bool)->Ops {Ops {action:if flag {|a,b|a.*} else {|a,b|b.*}}}",
            "let ops=make(flag);ops.action(left,right)",
            vec![0, 1],
        ),
        (
            "fn make(flag:bool)->fn(*bool,*bool)->bool {if flag {make(false)} else {|a:*bool,b:*bool|a.*}}",
            "let callback=make(flag);callback(left,right)",
            vec![0],
        ),
        (
            "fn keep(callback:fn(*bool,*bool)->bool)->fn(*bool,*bool)->bool {callback}",
            "let callback=keep(|a,b|a.*);callback(left,right)",
            vec![0],
        ),
        (
            "",
            "let make=|| {|a:*bool,b:*bool|a.*};let callback=make();callback(left,right)",
            vec![0],
        ),
        (
            "fn maker()->fn()->fn(*bool,*bool)->bool {|| {|a:*bool,b:*bool|a.*}}",
            "let make=maker();let callback=make();callback(left,right)",
            vec![0],
        ),
        (
            "fn wrap(action:fn(*bool,*bool)->bool)->%{action:fn(*bool,*bool)->bool} fn(*bool,*bool)->bool {|a:*bool,b:*bool|action(a,b)}",
            "let callback=wrap(if flag {|a,b|a.*} else {|a,b|b.*});callback(left,right)",
            vec![0, 1],
        ),
        (
            "fn make(flag:bool)->fn(*bool,*bool)->bool {if flag {return |a:*bool,b:*bool|a.*;} |a:*bool,b:*bool|b.*}",
            "let callback=make(flag);callback(left,right)",
            vec![0, 1],
        ),
    ] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {definitions}
            fn inspect(left:*bool,right:*bool,flag:bool)->bool {{{body}}}
            fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,true)}}
            fn run_false()->bool {{let left=true;let right=false;inspect(left.ref,right.ref,false)}}"));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary = memory::infer_in_package(&package, inspect.declaration, &[], &mut 400000)
            .unwrap_or_else(|e| panic!("{definitions}; {body}: {e}"));
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        let false_result = expected.len() == 1;
        assert_eq!(reads, expected.into_iter().collect(), "{body}: {summary}");
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                600000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run_false",
                "Duplicate",
                "Retire",
                600000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(false_result)
        );
        assert!(memory::infer_in_package(&package, inspect.declaration, &[], &mut 10).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 400000).unwrap()
        );
    }
}

#[test]
fn memory_returned_code_distinguishes_recursive_bottom_from_unknown() {
    use elaboration::verification::memory::{self, CodeResult};
    let package=compile("fn never()->fn(*bool)->bool {never()}
        fn choose(input:fn(*bool)->bool,flag:bool)->fn(*bool)->bool {if flag {input} else {|p:*bool|p.*}}
        fn inspect(p:*bool,input:fn(*bool)->bool,flag:bool)->bool {let callback=choose(input,flag);callback(p)}");
    let declaration = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let bottom =
        memory::infer_in_package(&package, declaration("never"), &[], &mut 100000).unwrap();
    assert!(matches!(bottom.code_fields[&vec![]], CodeResult::NoReturn));
    assert!(bottom.to_string().contains("(code-result no-return)"));
    let unknown =
        memory::infer_in_package(&package, declaration("choose"), &[], &mut 100000).unwrap();
    assert!(matches!(unknown.code_fields[&vec![]], CodeResult::Unknown));
    assert!(unknown.to_string().contains("(code-result unknown)"));
    assert!(
        memory::infer_in_package(&package, declaration("inspect"), &[], &mut 100000)
            .unwrap_err()
            .contains("memory contract")
    );
}

#[test]
fn memory_returned_anonymous_code_rejects_retired_actual_targets() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool} fn consume(value:Cell) {}
        fn make(flag:bool)->fn(*bool,*bool)->*bool {if flag {|a:*bool,b:*bool|a} else {|a:*bool,b:*bool|b}}
        fn inspect(a:*bool,b:*bool,flag:bool)->bool {let pointer=make(flag)(a,b);pointer.*}";
    for flag in [true, false] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,{flag})}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
    }
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let a=true;let b=Cell {{value:false}};let pointer=b.value.ref;consume(b);inspect(a.ref,pointer,false)}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_returned_anonymous_code_specializes_before_leaving_its_definition() {
    use elaboration::verification::memory;
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);} typealias Byte=.u8'builtin;
        fn make()->fn(*T)->*T where T {|p:*T|p}
        fn inspect(p:*T)->*T where T {let callback=make<T>();callback(p)}
        fn run()->bool {let flag=true;let byte:Byte=7;inspect(flag.ref).* and inspect(byte.ref).* == 7}");
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    for ty in [nir::Primitive::Bool, nir::Primitive::U8] {
        let summary = memory::infer_in_package(
            &package,
            inspect.declaration,
            &[nir::Term::Primitive(ty)],
            &mut 300000,
        )
        .unwrap();
        assert_eq!(
            summary
                .result_targets
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .parameter,
            0
        );
    }
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn memory_recursive_factories_return_live_scalar_environments() {
    use elaboration::verification::memory;
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn make(flag:bool,again:bool)->%{flag:bool} fn(*bool,*bool)->bool {
            if again {make(flag,false)} else {|a:*bool,b:*bool|if flag {a.*} else {b.*}}
        }
        fn inspect(a:*bool,b:*bool,flag:bool)->bool {let callback=make(flag,true);callback(a,b)}
        fn yes()->bool {let a=true;let b=false;inspect(a.ref,b.ref,true)}
        fn no()->bool {let a=true;let b=false;inspect(a.ref,b.ref,false)}",
    );
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 400000).unwrap();
    let reads = summary
        .accesses
        .iter()
        .filter(|a| a.read_value)
        .map(|a| a.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, std::collections::BTreeSet::from([0, 1]));
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                entry,
                "Duplicate",
                "Retire",
                600000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(expected)
        );
    }
}

#[test]
fn memory_recursive_returned_data_preserves_copy_and_lifetime_boundaries() {
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        struct Inner {flag:bool} impl Duplicate for Inner {}
        struct Config {inner:Inner} impl Duplicate for Config {}
        fn make(config:Config,again:bool)->%{config:Config} fn(*bool,*bool)->bool {
            if again {make(config,false)} else {|a:*bool,b:*bool|if config.inner.flag {a.*} else {b.*}}
        }
        fn inspect(a:*bool,b:*bool,flag:bool)->bool {let callback=make(Config {inner:Inner {flag:flag}},true);callback(a,b)}";
    for flag in [true, false] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,{flag})}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                800000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
    }
    let package=compile(&format!("{prefix}
        struct Cell {{value:bool}} fn consume(cell:Cell) {{}}
        fn run()->bool {{let a=true;let b=Cell {{value:false}};let p=b.value.ref;consume(b);inspect(a.ref,p,false)}}"));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
    // An outer Copy declaration cannot authorize an affine nested field.
    let invalid = prefix.replace("impl Duplicate for Inner {}", "");
    let package = compile(&format!(
        "{invalid} fn run()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,true)}}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 800000)
            .unwrap_err();
    assert!(error.contains("Copy"), "{error}");
}

#[test]
fn memory_recursive_factories_retain_copy_callback_captures() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Action=fn(*bool,*bool)->bool; impl Duplicate for Action {}
        fn first(a:*bool,b:*bool)->bool {a.*}
        fn second(a:*bool,b:*bool)->bool {b.*}
        fn wrap(action:Action,again:bool)->%{action:Action} fn(*bool,*bool)->bool {
            if again {wrap(action,false)} else {|a:*bool,b:*bool|action(a,b)}
        }";
    for actual in [
        "if flag {first} else {second}",
        "if flag {|a,b|a.*} else {|a,b|b.*}",
    ] {
        let package=compile(&format!("{prefix}
            fn inspect(a:*bool,b:*bool,flag:bool)->bool {{let callback=wrap({actual},true);callback(a,b)}}
            fn yes()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,true)}}
            fn no()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,false)}}"));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 700000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, std::collections::BTreeSet::from([0, 1]));
        for (entry, expected) in [("yes", true), ("no", false)] {
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    entry,
                    "Duplicate",
                    "Retire",
                    1000000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(expected)
            );
        }
    }
}

#[test]
fn memory_recursive_returned_callback_fields_preserve_targets_and_snapshots() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Action=fn(*bool,*bool)->bool; impl Duplicate for Action {}
        struct Ops {action:Action,flag:bool} impl Duplicate for Ops {}
        fn wrap(ops:Ops,again:bool)->%{saved:Ops} fn(*bool,*bool)->bool {
            if again {wrap(ops,false)} else {var saved=ops;|a:*bool,b:*bool| {
                saved.flag=not saved.flag;
                if saved.flag {saved.action(a,b)} else {false}
            }}
        }
        fn inspect(a:*bool,b:*bool,flag:bool)->bool {
            var action:Action=if flag {|a,b|a.*} else {|a,b|b.*};
            let callback=wrap(Ops {action:action,flag:true},true);
            action=|a,b|false;
            callback(a,b);
            callback(a,b)
        }";
    let package = compile(&format!(
        "{prefix}
        fn yes()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,true)}}
        fn no()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,false)}}"
    ));
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 900000).unwrap();
    let reads = summary
        .accesses
        .iter()
        .filter(|a| a.read_value)
        .map(|a| a.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, std::collections::BTreeSet::from([0, 1]));
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                entry,
                "Duplicate",
                "Retire",
                1200000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(expected)
        );
    }
    let package=compile(&format!("{prefix}
        struct Cell {{value:bool}} fn consume(cell:Cell) {{}}
        fn run()->bool {{let a=true;let b=Cell {{value:false}};let p=b.value.ref;consume(b);inspect(a.ref,p,false)}}"));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1200000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn memory_recursive_returned_mutable_callbacks_use_a_checked_capture_invariant() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Action=fn(*bool,*bool)->bool; impl Duplicate for Action {}
        fn first(a:*bool,b:*bool)->bool {a.*} fn second(a:*bool,b:*bool)->bool {b.*}
        fn wrap(initial:Action,again:bool)->%{action:Action} fn(*bool,*bool)->bool {
            if again {wrap(initial,false)} else {var action=initial;|a:*bool,b:*bool| {let result=action(a,b);action=second;result}}
        }
        fn inspect(a:*bool,b:*bool)->bool {let callback=wrap(first,true);callback(a,b);callback(a,b)}
        fn run()->bool {let a=true;let b=false;inspect(a.ref,b.ref)}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1200000)
            .unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
}

#[test]
fn dynamic_bit_writeback_tracks_selection_and_rhs_ownership() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("let i:Byte=2;p.word'bitvec[i]=0b1;", true),
        ("p.word'bitvec[consume(p)]=0b1;", false),
        ("let i:Byte=2;p.word'bitvec[i]=do {let moved=p;0b1};", false),
        (
            "let i:Byte=2;p.word'bitvec[i]=do {let moved=p;return;};",
            true,
        ),
    ] {
        let package = compile(&format!(
            "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;
            struct Packet {{word:Word,}}
            fn consume(value:Packet)->result:Byte where ensures result == 2 {{2}}
            fn run() {{let p=Packet {{word:4660}};{body}}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn dynamic_bit_writeback_memory_summary_includes_positions_and_rhs() {
    use elaboration::verification::memory;
    let package = compile(
        "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;
        fn edit(p:*Word,i:*Byte,q:*Word)->Word {var word=p.*;let start=i.*;assumes start <= 8;
            word'bitvec[start..(start + 8)]=(q.*)'bitvec[0..8];word}",
    );
    let function = package.functions.iter().find(|f| f.name == "edit").unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert_eq!(
        summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "{summary}"
    );
    assert!(
        summary.accesses.iter().all(|a| !a.write),
        "local bit update escaped into caller memory: {summary}"
    );
}

#[test]
fn pointer_bit_writeback_requires_the_original_owner_after_positions_and_rhs() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("let i:Byte=4;p.*'bitvec[i..(i + 4)]=0b1010;", true),
        ("p.*'bitvec[consume(packet)]=1;", false),
        ("let i:Byte=4;p.*'bitvec[i]=do {let moved=packet;1};", false),
        (
            "let i:Byte=4;p.*'bitvec[i]=do {let moved=packet;return;};",
            true,
        ),
        ("let i:Byte=4;p.*'bitvec[i]=do {let moved=other;1};", true),
        (
            "let i:Byte=4;p.*'bitvec[i]=do {p=other.word.ref;let moved=packet;1};",
            false,
        ),
    ] {
        let package=compile(&format!("typealias Word=.u16'builtin;typealias Byte=.u8'builtin;struct Packet {{word:Word,}}
            fn consume(value:Packet)->result:Byte where ensures result == 4 {{4}}
            fn run() {{let packet=Packet {{word:0}};let other=Packet {{word:0}};var p=packet.word.ref;{body}}}"));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            run,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn pointer_bit_writeback_memory_summaries_track_read_modify_write_and_alias_rebinding() {
    use elaboration::verification::memory;
    let package=compile("typealias Word=.u16'builtin;typealias Byte=.u8'builtin;
        fn edit(p:*Word,i:*Byte,q:*Word) {let start=i.*;assumes start <= 8;p.*'bitvec[start..(start + 8)]=q.*'bitvec[0..8];}
        fn wrapper(p:*Word,i:*Byte,q:*Word) {edit(p,i,q);}
        fn swapped(p:*Word,q:*Word) {var target=p;let i:Byte=4;target.*'bitvec[i]=do {target=q;1};}");
    for entry in ["edit", "wrapper"] {
        let function = package.functions.iter().find(|f| f.name == entry).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert_eq!(
            summary
                .accesses
                .iter()
                .filter(|a| a.write)
                .map(|a| a.target.parameter)
                .collect::<Vec<_>>(),
            vec![0],
            "{summary}"
        );
        assert_eq!(
            summary
                .accesses
                .iter()
                .filter(|a| a.read_value)
                .map(|a| a.target.parameter)
                .collect::<Vec<_>>(),
            vec![1, 2, 0],
            "{summary}"
        );
    }
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "swapped")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert_eq!(
        summary
            .accesses
            .iter()
            .filter(|a| a.write)
            .map(|a| a.target.parameter)
            .collect::<Vec<_>>(),
        vec![0],
        "pointer rebinding changed the prepared destination: {summary}"
    );
}

#[test]
fn conditional_variant_patterns_preserve_all_moved_pointer_origins() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for expression in [
        "if flag {E.left(Holder {ptr:first.value.ref},false)} else {E.right(false,Holder {ptr:second.value.ref})}",
        "flag match {true=>E.left(Holder {ptr:first.value.ref},false),false=>E.right(false,Holder {ptr:second.value.ref})}",
    ] {
        for moved in ["", "let moved=first;", "let moved=second;"] {
            let package = compile(&format!(
                "struct Cell {{value:bool}} struct Holder {{ptr:*bool}}
             enum E {{left(Holder,bool),right(bool,Holder)}}
             fn run(flag:bool) {{let first=Cell {{value:true}};let second=Cell {{value:false}};
                let value={expression};
                let (.left(item,_) or .right(_,item))=value;
                {moved} let seen=item.ptr.*;
             }}"
            ));
            let run = package
                .functions
                .iter()
                .find(|function| function.name == "run")
                .unwrap();
            let result = analyze_in_package(
                &package,
                run,
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
            );
            assert_eq!(
                result.is_ok(),
                moved.is_empty(),
                "{expression}, {moved}: {result:?}"
            );
        }
    }
}

#[test]
fn conditional_variant_patterns_do_not_erase_unknown_pointer_sources() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    let package = compile(
        "struct Cell {value:bool} struct Holder {ptr:*bool}
        enum E {left(Holder,bool),right(bool,Holder)}
        fn run(flag:bool,supplied:E) {let cell=Cell {value:true};
            let value=if flag {E.left(Holder {ptr:cell.value.ref},false)} else {supplied};
            let (.left(item,_) or .right(_,item))=value;let seen=item.ptr.*;
        }",
    );
    let run = package
        .functions
        .iter()
        .find(|function| function.name == "run")
        .unwrap();
    let result = analyze_in_package(
        &package,
        run,
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
    );
    let failure = result.unwrap_err();
    assert!(
        failure.reason.contains("pointer read target unavailable"),
        "{failure}"
    );
}

#[test]
fn conditional_effect_memory_tracks_forwarded_pointer_results() {
    use elaboration::verification::memory;
    let package = compile(
        "effect relay(p:*bool)->*bool;
        fn subject(p:*bool)->#relay *bool {relay(p)#}
        fn forward(p:*bool)->#relay *bool {subject(p)}
        fn leak()->#relay *bool {var flag=true;relay(flag.ref)#}",
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
        forwarded_parameters: Some(std::collections::BTreeSet::from([0])),
    };
    for name in ["subject", "forward"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let infer = |input: memory::EffectInput, fuel: &mut usize| {
            memory::infer_with_effect_inputs(
                &package,
                function.declaration,
                &[],
                &Default::default(),
                &Default::default(),
                &[input],
                fuel,
            )
        };
        let summary = infer(input.clone(), &mut 100_000).unwrap();
        let targets = summary.result_targets.as_ref().unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .all(|access| !access.read_value && !access.write)
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(targets.iter().next().unwrap().parameter, 0);
        assert!(summary.to_string().contains("(forwarded-parameters 0)"));
        let mut wrong = input.clone();
        wrong.forwarded_parameters = None;
        assert!(infer(wrong, &mut 100_000).is_err());
        let mut wrong = input.clone();
        wrong.forwarded_parameters = Some(std::collections::BTreeSet::from([1]));
        assert!(infer(wrong, &mut 100_000).is_err());
        assert!(infer(input.clone(), &mut 0).is_err());
        assert_eq!(summary, infer(input.clone(), &mut 100_000).unwrap());
    }
    let leak = package.functions.iter().find(|f| f.name == "leak").unwrap();
    let failure = memory::infer_with_effect_inputs(
        &package,
        leak.declaration,
        &[],
        &Default::default(),
        &Default::default(),
        &[input],
        &mut 100_000,
    )
    .unwrap_err();
    assert!(failure.contains("escape"), "{failure}");
}

#[test]
fn conditional_effect_memory_preserves_footprints_and_premises() {
    use elaboration::verification::memory;
    let package = compile(
        "effect poll(value:bool)->bool;
        fn inner(p:*bool)->#poll bool {poll(p.*)#}
        fn subject(p:*bool)->#poll bool {inner(p)}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let operation = &package.effects[0];
    let input = memory::EffectInput {
        parameter_accesses: Default::default(),
        owned_inputs: false,
        copy_parameters: Default::default(),
        forwarded_parameters: None,
        operation: nir::effects::Operation {
            declaration: operation.declaration,
            arguments: vec![],
        },
        abi: operation.abi,
        signature: operation.signature.clone(),
    };
    let infer = |inputs: &[memory::EffectInput], fuel: &mut usize| {
        memory::infer_with_effect_inputs(
            &package,
            function.declaration,
            &[],
            &Default::default(),
            &Default::default(),
            inputs,
            fuel,
        )
    };
    assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 100000).is_err());
    let summary = infer(&[input.clone()], &mut 100000).unwrap();
    assert_eq!(summary.effect_inputs, vec![input.clone()]);
    assert!(summary.may_reject);
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.read_value)
    );
    assert!(summary.accesses.iter().all(|a| !a.write));
    assert!(summary.to_string().contains("(effect-inputs (effect-input"));
    assert!(
        infer(&[], &mut 100000)
            .unwrap_err()
            .contains("handler memory premise")
    );
    assert!(infer(&[input.clone()], &mut 1).is_err());
    assert_eq!(summary, infer(&[input.clone()], &mut 100000).unwrap());
    let mut wrong = input.clone();
    wrong.abi = nir::effects::Abi::DirectPassing;
    assert!(infer(&[wrong], &mut 100000).is_err());
    let mut wrong = input;
    wrong.operation.declaration.local += 1000;
    assert!(infer(&[wrong], &mut 100000).is_err());
}

#[test]
fn conditional_effect_memory_checks_instance_identity_and_routing() {
    use elaboration::verification::memory;
    use nir::program::{ExprKind as K, Rewriter};
    let package = compile(
        "effect choose()->T where T;
        fn subject()->#[choose<bool>,choose<.char'builtin>] bool {
            (choose<bool>()#) and (choose<.char'builtin>()#)=='x'
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let operation = &package.effects[0];
    let inputs = [nir::Primitive::Bool, nir::Primitive::Char]
        .into_iter()
        .map(|p| {
            let arguments = vec![nir::Term::Primitive(p)];
            memory::EffectInput {
                parameter_accesses: Default::default(),
                owned_inputs: false,
                copy_parameters: Default::default(),
                forwarded_parameters: None,
                operation: nir::effects::Operation {
                    declaration: operation.declaration,
                    arguments: arguments.clone(),
                },
                abi: operation.abi,
                signature: operation
                    .signature
                    .instantiate_types(&operation.type_parameters, &arguments)
                    .unwrap(),
            }
        })
        .collect::<Vec<_>>();
    let infer = |package: &nir::program::Package, inputs: &[memory::EffectInput]| {
        memory::infer_with_effect_inputs(
            package,
            function.declaration,
            &[],
            &Default::default(),
            &Default::default(),
            inputs,
            &mut 100000,
        )
    };
    assert!(infer(&package, &inputs).is_ok());
    assert!(
        infer(&package, &inputs[..1]).is_err(),
        "same declaration must not authorize a different instance"
    );
    struct BreakSlot;
    impl Rewriter for BreakSlot {
        type Error = String;
        fn ty(&mut self, ty: &nir::Term) -> Result<nir::Term, String> {
            Ok(ty.clone())
        }
        fn expression(&mut self, expr: &mut nir::program::Expr) -> Result<bool, String> {
            if let K::EffectRequest { allowed_index, .. } = &mut expr.kind {
                *allowed_index = 999;
            }
            Ok(true)
        }
    }
    let mut invalid = package.clone();
    invalid
        .functions
        .iter_mut()
        .find(|f| f.name == "subject")
        .unwrap()
        .body
        .rewrite_with(&mut BreakSlot)
        .unwrap();
    assert!(
        infer(&invalid, &inputs)
            .unwrap_err()
            .contains("row mismatch")
    );
    assert!(
        infer(&package, &inputs).is_ok(),
        "failed inference must not contaminate later calls"
    );
}

#[test]
fn conditional_effect_memory_does_not_erase_pointer_delivery() {
    use elaboration::verification::memory;
    for source in [
        "effect accept(value:*bool)->bool;fn subject(p:*bool)->#accept bool {accept(p)#}",
        "effect address()->*bool;fn subject()->#address *bool {address()#}",
    ] {
        let package = compile(source);
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let operation = &package.effects[0];
        let input = memory::EffectInput {
            parameter_accesses: Default::default(),
            owned_inputs: false,
            copy_parameters: Default::default(),
            forwarded_parameters: None,
            operation: nir::effects::Operation {
                declaration: operation.declaration,
                arguments: vec![],
            },
            abi: operation.abi,
            signature: operation.signature.clone(),
        };
        let error = memory::infer_with_effect_inputs(
            &package,
            function.declaration,
            &[],
            &Default::default(),
            &Default::default(),
            &[input],
            &mut 100000,
        )
        .unwrap_err();
        assert!(error.contains("data-only direct delivery"), "{error}");
    }
}

#[test]
fn conditional_effect_closure_memory_retains_virtual_capture_accesses() {
    use elaboration::verification::memory;
    for body in [
        "pointer.* = true;poll(pointer.*)#",
        "pointer.* = true;forward(pointer.*)",
    ] {
        let package = compile(&format!(
            "effect poll(value:bool)->bool;
            fn forward(value:bool)->#poll bool {{poll(value)#}}
            typealias Subject=%{{pointer:*bool}} fn()->#poll bool;
            fn factory(pointer:*bool)->Subject {{|| {{{body}}}}}"
        ));
        let factory = package
            .functions
            .iter()
            .find(|f| f.name == "factory")
            .unwrap();
        let mut definitions = vec![];
        nir::visit::expressions(factory, &mut 100000, |expr, _| {
            if matches!(expr.kind, nir::program::ExprKind::Closure { .. }) {
                definitions.push(expr.clone());
            }
            Ok(())
        })
        .unwrap();
        let definition = &definitions[0];
        let operation = &package.effects[0];
        let input = memory::EffectInput {
            parameter_accesses: Default::default(),
            owned_inputs: false,
            copy_parameters: Default::default(),
            forwarded_parameters: None,
            operation: nir::effects::Operation {
                declaration: operation.declaration,
                arguments: vec![],
            },
            abi: operation.abi,
            signature: operation.signature.clone(),
        };
        let infer = |effects: &[memory::EffectInput], fuel: &mut usize| {
            memory::infer_closure_with_effect_inputs(
                &package,
                definition,
                &Default::default(),
                &Default::default(),
                &Default::default(),
                &Default::default(),
                effects,
                fuel,
            )
        };
        let summary = infer(&[input.clone()], &mut 200000).unwrap();
        assert_eq!(summary.effect_inputs, vec![input.clone()]);
        assert!(
            summary
                .memory
                .accesses
                .iter()
                .any(|a| a.target.parameter == 0 && a.write),
            "{summary}"
        );
        assert!(
            summary
                .memory
                .accesses
                .iter()
                .any(|a| a.target.parameter == 0 && a.read_value),
            "{summary}"
        );
        assert!(summary.memory.may_reject);
        assert!(
            summary
                .to_string()
                .starts_with("(conditional-closure-memory (effect-inputs")
        );
        assert!(infer(&[], &mut 200000).is_err());
        assert!(
            memory::infer_closure_with_borrowed_captures(
                &package,
                definition,
                &Default::default(),
                &Default::default(),
                &Default::default(),
                &Default::default(),
                &mut 200000
            )
            .is_err()
        );
        assert!(infer(&[input.clone()], &mut 1).is_err());
        assert_eq!(summary, infer(&[input], &mut 200000).unwrap());
    }
}

#[test]
fn conditional_effect_closure_memory_orders_parameters_before_captures_and_rejects_slot_exposure() {
    use elaboration::verification::memory;
    for expose in [false, true] {
        let prefix = if expose { "let slot=pointer.ref;" } else { "" };
        let package=compile(&format!("effect poll(value:bool)->bool;
            typealias Subject=%{{pointer:*bool}} fn(*bool)->#poll bool;
            fn factory(pointer:*bool)->Subject {{|other:*bool| {{{prefix}pointer.* = other.*;poll(pointer.*)#}}}}"));
        let factory = package
            .functions
            .iter()
            .find(|f| f.name == "factory")
            .unwrap();
        let mut definitions = vec![];
        nir::visit::expressions(factory, &mut 100000, |expr, _| {
            if matches!(expr.kind, nir::program::ExprKind::Closure { .. }) {
                definitions.push(expr.clone());
            }
            Ok(())
        })
        .unwrap();
        let operation = &package.effects[0];
        let input = memory::EffectInput {
            parameter_accesses: Default::default(),
            owned_inputs: false,
            copy_parameters: Default::default(),
            forwarded_parameters: None,
            operation: nir::effects::Operation {
                declaration: operation.declaration,
                arguments: vec![],
            },
            abi: operation.abi,
            signature: operation.signature.clone(),
        };
        let result = memory::infer_closure_with_effect_inputs(
            &package,
            &definitions[0],
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &[input],
            &mut 200000,
        );
        if expose {
            assert!(
                result.is_err(),
                "effect premises must not authorize captured pointer-slot exposure"
            );
        } else {
            let summary = result.unwrap();
            assert!(
                summary
                    .memory
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 0 && a.read_value),
                "{summary}"
            );
            assert!(
                summary
                    .memory
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 1 && a.write),
                "{summary}"
            );
            assert!(
                summary
                    .memory
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 1 && a.read_value),
                "{summary}"
            );
            assert!(
                !summary
                    .memory
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 0 && a.write),
                "{summary}"
            );
        }
    }
}

#[test]
fn conditional_effect_data_delivery_preserves_argument_reads_and_rejects_hidden_aliases() {
    use elaboration::verification::memory;
    for (source, valid) in [
        (
            "struct Packet {value:bool} effect exchange(packet:Packet)->Packet;fn subject(p:*bool)->#exchange Packet {exchange(Packet {value:p.*})#}",
            true,
        ),
        (
            "struct Packet {value:*bool} effect exchange(packet:Packet)->Packet;fn subject(p:*bool)->#exchange Packet {exchange(Packet {value:p})#}",
            false,
        ),
        (
            "enum Reply {none,some(*bool)} effect receive()->Reply;fn subject()->#receive Reply {receive()#}",
            false,
        ),
        (
            "typealias Callback=fn()->bool;struct Reply {callback:Callback} effect receive()->Reply;fn subject()->#receive Reply {receive()#}",
            false,
        ),
    ] {
        let package = compile(source);
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let operation = &package.effects[0];
        let input = memory::EffectInput {
            parameter_accesses: Default::default(),
            owned_inputs: false,
            copy_parameters: Default::default(),
            forwarded_parameters: None,
            operation: nir::effects::Operation {
                declaration: operation.declaration,
                arguments: vec![],
            },
            abi: operation.abi,
            signature: operation.signature.clone(),
        };
        let infer = |inputs: &[memory::EffectInput], fuel: &mut usize| {
            memory::infer_with_effect_inputs(
                &package,
                function.declaration,
                &[],
                &Default::default(),
                &Default::default(),
                inputs,
                fuel,
            )
        };
        let result = infer(&[input.clone()], &mut 100000);
        if !valid {
            assert!(result.unwrap_err().contains("data-only direct delivery"));
            continue;
        }
        let summary = result.unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value)
        );
        assert!(summary.may_reject);
        assert_eq!(summary.effect_inputs, vec![input.clone()]);
        assert!(infer(&[], &mut 100000).is_err());
        assert!(infer(&[input.clone()], &mut 1).is_err());
        assert_eq!(summary, infer(&[input], &mut 100000).unwrap());
    }
}

#[test]
fn local_handler_memory_summary_retains_captured_pointer_accesses_without_exporting_local_premises()
{
    use elaboration::verification::memory;
    let package=compile("effect poll(value:bool)->bool;
        fn subject()->#poll bool {poll(true)#}
        fn handled(pointer:*bool)->bool {subject()# {poll(value)=>do {pointer.* = value;pointer.*},}}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "handled")
        .unwrap();
    let infer =
        |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
    let summary = infer(&mut 100000).unwrap();
    assert!(summary.effect_inputs.is_empty(), "{summary}");
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 0 && access.write)
    );
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 0 && access.read_value)
    );
    assert!(summary.may_reject);
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 100000).unwrap());
}

#[test]
fn local_handler_memory_summary_rejects_capture_slot_changes_and_unknown_fallback() {
    use elaboration::verification::memory;
    for (source, name, expected) in [
        (
            "effect poll()->bool;fn subject()->#poll bool {poll()#} fn handled(p:*bool,q:*bool)->bool {var pointer=p;do {pointer=q;subject()}# {poll()=>pointer.*,}}",
            "handled",
            "captured pointer rebinding",
        ),
        (
            "effect poll()->bool;fn subject()->#poll bool {poll()#} fn handled()->#poll bool {subject()# {poll() if false=>true,}}",
            "handled",
            "handler memory premise",
        ),
    ] {
        let package = compile(source);
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let error =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn handler_borrowed_parameter_report_retains_conditional_pointee_premise() {
    use elaboration::verification::moves::{self, Delivery};
    let package = compile(
        "effect poll()->bool;fn subject()->#poll bool {poll()#}
        fn handled(pointer:*bool)->bool {subject()# {poll()=>do {pointer.* = true;pointer.*},}}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "handled")
        .unwrap();
    assert!(moves::analyze_in_package(&package, function, |_| Ok(Delivery::Copy), 100000).is_err());
    let borrows = vec![(0, nir::Term::Primitive(nir::Primitive::Bool))];
    let report = moves::analyze_in_package_with_borrows(
        &package,
        function,
        |_| Ok(Delivery::Copy),
        100000,
        None,
        &borrows,
    )
    .unwrap();
    assert_eq!(report.borrowed_parameters, borrows);
    assert!(
        report
            .to_string()
            .contains("(borrowed-parameters (borrowed-parameter (slot 0)")
    );
    assert!(
        moves::analyze_in_package_with_borrows(
            &package,
            function,
            |_| Ok(Delivery::Copy),
            100000,
            None,
            &[(
                0,
                nir::Term::Pointer(Box::new(nir::Term::Primitive(nir::Primitive::Bool)))
            )]
        )
        .is_err()
    );
}

#[test]
fn data_match_memory_summary_retains_both_pointer_result_origins() {
    use elaboration::verification::memory;
    let package=compile("enum Choice {left,right}
        fn choose(choice:Choice,left:*bool,right:*bool)->*bool {choice match {.left=>left,.right=>right}}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "choose")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    let targets = summary.result_targets.unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|target| target.parameter)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(
        targets
            .iter()
            .all(|target| target.fields.is_empty() && target.through.is_empty())
    );
}

#[test]
fn data_match_memory_summary_tracks_guard_reads_and_generic_payload_writes() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Packet where T {value:T}
        enum Reply where T {none,some(Packet<T>)}
        fn subject(reply:Reply<bool>,gate:*bool,output:*bool)->bool {
            reply match {
                .some({value}) if gate.* => do {output.* = value;value},
                _=>false
            }
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let infer =
        |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
    let summary = infer(&mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 1 && a.read_value),
        "{summary}"
    );
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 2 && a.write),
        "{summary}"
    );
    assert!(
        !summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 1 && a.write),
        "{summary}"
    );
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 100000).unwrap());
}

#[test]
fn data_match_memory_summary_tracks_enum_pointers_and_rejects_unknown_callbacks() {
    use elaboration::verification::memory;
    for (source, expected) in [
        (
            "enum Choice {none,some(*bool)} fn subject(value:Choice)->bool {value match {.none=>false,.some(pointer)=>pointer.*}}",
            Some(vec![usize::MAX, 1, 0]),
        ),
        (
            "typealias Callback=fn()->bool;enum Choice {none,some(Callback)} fn subject(value:Choice)->bool {value match {.none=>false,.some(callback)=>callback()}}",
            None,
        ),
        (
            "enum Choice {none,some(*bool)} fn subject(pointer:*bool)->bool {Choice.some(pointer) match {.none=>false,.some(value)=>value.*}}",
            Some(vec![]),
        ),
    ] {
        let package = compile(source);
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let result = memory::infer_in_package(&package, function.declaration, &[], &mut 100000);
        if let Some(fields) = expected {
            let summary = result.unwrap();
            assert_eq!(summary.accesses.len(), 1, "{summary}");
            let access = &summary.accesses[0];
            assert_eq!(access.target.parameter, 0);
            assert_eq!(access.target.input_fields, fields);
            assert!(access.read_value && !access.write, "{summary}");
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("dynamic call requires a richer memory contract"),
                "{error}"
            );
        }
    }
}

#[test]
fn computed_data_match_summary_retains_candidate_reads_and_branch_origins() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Packet {flag:bool}
        fn check(flag:bool)->bool {flag}
        fn subject(packet:Packet,gate:*bool,left:*bool,right:*bool)->*bool {
            packet match {
                item and check(item.flag and gate.*) is true=>left,
                _=>right
            }
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let infer =
        |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
    let summary = infer(&mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 1 && a.read_value),
        "{summary}"
    );
    assert!(!summary.accesses.iter().any(|a| a.write), "{summary}");
    assert_eq!(
        summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|t| t.parameter)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 100000).unwrap());
}

#[test]
fn computed_data_match_let_else_summary_retains_failure_return_origin() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Packet {flag:bool}
        fn subject(packet:Packet,gate:*bool,left:*bool,right:*bool)->*bool {
            let item and (item.flag and gate.*) is true = packet else {return right;};
            left
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 1 && a.read_value),
        "{summary}"
    );
    assert_eq!(
        summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|t| t.parameter)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[test]
fn guard_memory_summary_tracks_nested_reads_writes_and_region_boundaries() {
    use elaboration::verification::memory;
    for (guard, valid) in [
        ("accept(accept(gate.*))", true),
        ("accept(do {let seen=gate.*;accept(seen)})", true),
        (
            "do {let outer=gate.*;accept(do {var inner=not outer;inner=not inner;inner})}",
            true,
        ),
        (
            "do {let seen=gate.*;var answer=not seen;answer=not answer;accept(answer)}",
            true,
        ),
        ("produce()(gate.*)", true),
        ("with_default(gate.*)", true),
        (
            "do {let _ and produce() is action=false;action(gate.*)}",
            true,
        ),
        (
            "(gate.* match {true=>produce(),false=>produce()})(gate.*)",
            true,
        ),
        (
            "do {var action=produce();action=produce();action(gate.*)}",
            true,
        ),
        ("do {gate.* = true;true}", false),
        ("do {outside=true;true}", false),
        ("do {let pointer=gate;pointer.*}", true),
        (
            "gate.* match {true=>do {outside=true;true},false=>false}",
            false,
        ),
        (
            "do {let _ and do {outside=true;true} is seen=false;seen}",
            false,
        ),
    ] {
        let package=compile(&format!("struct Packet {{flag:bool}} fn accept(flag:bool)->bool {{flag}}
            typealias Action=fn(bool)->bool;fn produce()->Action {{accept}}
            fn with_default(value:bool,.selected:bool=accept(value))->bool {{selected}}
            fn subject(packet:Packet,gate:*bool)->bool {{var outside=false;packet match {{item if {guard}=>item.flag,_=>false}}}}"));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let result = memory::infer_in_package(&package, function.declaration, &[], &mut 100000);
        if valid {
            let summary = result.unwrap();
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 1 && a.read_value),
                "{summary}"
            );
            assert!(!summary.accesses.iter().any(|a| a.write), "{summary}");
        } else if guard == "do {gate.* = true;true}" {
            let summary = result.unwrap();
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 1 && a.write),
                "{summary}"
            );
        } else {
            assert!(result.unwrap_err().contains("memory guard"));
        }
    }
}

#[test]
fn later_pattern_mismatch_keeps_prior_pointer_observations() {
    use elaboration::verification::memory;
    for (ty, right, pattern) in [("bool", "true", "false"), ("Choice", "Choice.yes", ".no")] {
        let package = compile(&format!(
            "enum Choice {{yes,no}} struct Pair {{left:bool,right:{ty}}}
            fn subject(gate:*bool)->bool {{Pair {{left:true,right:{right}}} match {{
                {{left:_ and gate.* is _,right:{pattern}}}=>false,_=>true
            }}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value),
            "{summary}"
        );
        assert!(
            !summary.accesses.iter().any(|access| access.write),
            "{summary}"
        );
    }
}

#[test]
fn memory_summaries_stop_at_proven_abrupt_completion() {
    use elaboration::verification::memory;
    for (body, targets, writes) in [
        (
            "if true {return left;}right.* = true;assumes false;right",
            vec![1],
            false,
        ),
        (
            "if false {right.* = true;assumes false;return right;}left",
            vec![1],
            false,
        ),
        (
            "if flag {return left;}else{return right;}right.* = true;assumes false;right",
            vec![1, 2],
            false,
        ),
        (
            "if flag {return left;}right.* = true;right",
            vec![1, 2],
            true,
        ),
    ] {
        let package = compile(&format!(
            "fn subject(flag:bool,left:*bool,right:*bool)->*bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
        assert!(!summary.may_reject, "{body}: {summary}");
        assert_eq!(
            summary.accesses.iter().any(|a| a.write),
            writes,
            "{body}: {summary}"
        );
        let actual = summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|t| t.parameter)
            .collect::<Vec<_>>();
        assert_eq!(actual, targets, "{body}: {summary}");
    }
}

#[test]
fn inline_guard_return_regions_preserve_pointer_read_footprints() {
    use elaboration::verification::memory;
    for modifier in ["inline", "quote"] {
        for body in [
            "if value {return true;}false",
            "tick();if value {return true;}false",
            "let unit=tick();if value {return true;}false",
            "let true=value else {return false;};true",
            "let _ and value is chosen=false;if chosen {return true;}false",
        ] {
            let original = compile(&format!(
                "struct Packet {{flag:bool}}
            fn tick() {{}}
            {modifier} fn accept(value:bool)->bool {{{body}}}
            fn subject(packet:Packet,gate:*bool)->bool {{
                packet match {{item if accept(gate.*)=>item.flag,_=>false}}
            }}"
            ));
            for expanded in [false, true] {
                let mut package = original.clone();
                if expanded {
                    elaboration::program::inlining::expand(&mut package, &mut 1000000).unwrap();
                }
                let function = package
                    .functions
                    .iter()
                    .find(|f| f.name == "subject")
                    .unwrap();
                let summary =
                    memory::infer_in_package(&package, function.declaration, &[], &mut 100000)
                        .unwrap();
                assert!(
                    summary
                        .accesses
                        .iter()
                        .any(|a| a.target.parameter == 1 && a.read_value),
                    "{summary}"
                );
                assert!(!summary.accesses.iter().any(|a| a.write), "{summary}");
                assert!(!summary.may_reject, "{summary}");
                assert!(
                    memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err()
                );
            }
        }
    }
}

#[test]
fn handler_guard_summary_keeps_reads_and_writes_without_leaking_local_premises() {
    use elaboration::verification::memory;
    let package=compile("fn accept(value:bool)->bool {value}
        effect poll(value:bool)->bool;fn request()->#poll bool {poll(true)#}
        fn handled(gate:*bool,output:*bool)->bool {
            (request()# {poll(value) if do {let observed=gate.*;accept(observed and value)}=>do {output.* = true;true},})#
                {poll(value)=>false,}
        }");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "handled")
        .unwrap();
    let infer =
        |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
    let summary = infer(&mut 200000).unwrap();
    assert!(summary.effect_inputs.is_empty(), "{summary}");
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.read_value),
        "{summary}"
    );
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 1 && a.write),
        "{summary}"
    );
    assert!(
        !summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.write),
        "{summary}"
    );
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 200000).unwrap());
}

#[test]
fn enum_callable_match_summary_joins_guarded_results_without_losing_identity() {
    use elaboration::verification::memory::{self, CallableResult};
    let package=compile("enum Choice {first(fn(bool)->bool),second(fn(bool)->bool)}
        fn yes(value:bool)->bool {true} fn no(value:bool)->bool {false}
        fn choose(first:bool)->Choice {if first {Choice.first(yes)} else {Choice.second(no)}}
        fn extract(first:bool)->fn(bool)->bool {choose(first) match {.first(action)=>action,.second(action)=>action}}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "extract")
        .unwrap();
    let infer =
        |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
    let summary = infer(&mut 200000).unwrap();
    let CallableResult::Candidates(keys) = &summary.callable_result else {
        panic!("{summary}")
    };
    assert_eq!(keys.len(), 2);
    for name in ["yes", "no"] {
        let declaration = package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration;
        assert!(
            keys.iter().any(|k| k.declaration == declaration),
            "{summary}"
        );
    }
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 200000).unwrap());
}

#[test]
fn enum_callable_match_summary_or_patterns_merge_identities_and_unknown_is_not_bottom() {
    use elaboration::verification::memory::{self, CallableResult};
    let package=compile("enum Choice {first(fn(bool)->bool),second(fn(bool)->bool)}
        fn yes(value:bool)->bool {true} fn no(value:bool)->bool {false}
        fn choose(first:bool)->Choice {if first {Choice.first(yes)} else {Choice.second(no)}}
        fn extract(first:bool)->fn(bool)->bool {choose(first) match {(.first(action) or .second(action))=>action}}
        fn invoke(first:bool,action:fn(bool)->bool)->bool {
            let choice=if first {Choice.first(action)} else {Choice.second(no)};
            choice match {.first(callback)=>callback(true),.second(callback)=>callback(false)}
        }");
    let function = |name: &str| package.functions.iter().find(|f| f.name == name).unwrap();
    let summary =
        memory::infer_in_package(&package, function("extract").declaration, &[], &mut 200000)
            .unwrap();
    assert!(
        matches!(summary.callable_result,CallableResult::Candidates(ref keys) if keys.len()==2),
        "{summary}"
    );
    let fields =
        memory::infer_in_package(&package, function("choose").declaration, &[], &mut 200000)
            .unwrap();
    assert!(fields.to_string().contains("(variant 0) 0"), "{fields}");
    assert!(fields.to_string().contains("(variant 1) 0"), "{fields}");
    assert!(
        memory::infer_in_package(&package, function("invoke").declaration, &[], &mut 200000)
            .is_err(),
        "a potentially active unknown callback cannot be treated as an absent variant"
    );
}

#[test]
fn enum_callable_memory_contracts_preserve_variant_paths_accesses_and_pointer_results() {
    use elaboration::verification::memory::{self, CallableInput, CallableResult};
    use std::collections::BTreeMap;
    let package=compile("enum Choice {left(fn(*bool,*bool)->*bool),right(fn(*bool,*bool)->*bool)}
        fn left(p:*bool,q:*bool)->*bool {p.* = true;p}
        fn right(p:*bool,q:*bool)->*bool {let observed=q.*;q}
        fn invoke(choice:Choice,p:*bool,q:*bool)->*bool {choice match {.left(action)=>action(p,q),.right(action)=>action(p,q)}}");
    let function = |name: &str| package.functions.iter().find(|f| f.name == name).unwrap();
    let key = |name: &str| {
        CallableResult::Known(iir::InstanceKey {
            declaration: function(name).declaration,
            arguments: vec![],
        })
    };
    let inputs = BTreeMap::from([
        (
            CallableInput {
                parameter: 0,
                fields: vec![usize::MAX, 0, 0],
            },
            key("left"),
        ),
        (
            CallableInput {
                parameter: 0,
                fields: vec![usize::MAX, 1, 0],
            },
            key("right"),
        ),
    ]);
    let infer = |inputs: &BTreeMap<CallableInput, CallableResult>, fuel: &mut usize| {
        memory::infer_with_inputs(&package, function("invoke").declaration, &[], inputs, fuel)
    };
    let summary = infer(&inputs, &mut 200000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 1 && a.write),
        "{summary}"
    );
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 2 && a.read_value),
        "{summary}"
    );
    assert!(
        !summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 2 && a.write),
        "{summary}"
    );
    assert_eq!(
        summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|t| t.parameter)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(infer(&BTreeMap::new(), &mut 200000).is_err());
    for path in [
        vec![usize::MAX],
        vec![usize::MAX, 2, 0],
        vec![usize::MAX, 0, 1],
        vec![0],
    ] {
        assert!(
            infer(
                &BTreeMap::from([(
                    CallableInput {
                        parameter: 0,
                        fields: path
                    },
                    key("left")
                )]),
                &mut 200000
            )
            .is_err()
        );
    }
    assert!(infer(&inputs, &mut 1).is_err());
    assert_eq!(summary, infer(&inputs, &mut 200000).unwrap());
}

#[test]
fn memory_enum_capture_updates_include_new_variant_callbacks() {
    use elaboration::verification::memory;
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Action=fn(*bool,*bool)->bool; impl Duplicate for Action {}
        enum Ops {left(Action),right(Action)} impl Duplicate for Ops {}
        fn first(a:*bool,b:*bool)->bool {a.*} fn second(a:*bool,b:*bool)->bool {b.*}
        fn inspect(left:*bool,right:*bool)->bool {
            var ops=Ops.left(first);
            let callback=|a:*bool,b:*bool| {let result=ops match {.left(action)=>action(a,b),.right(action)=>action(a,b)};ops=Ops.right(second);result};
            callback(left,right);callback(left,right)
        }
        fn run()->bool {let left=true;let right=false;inspect(left.ref,right.ref)}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let infer =
        |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
    let summary = infer(&mut 300000).unwrap();
    assert_eq!(
        summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([0, 1]),
        "{summary}"
    );
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 300000).unwrap());
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
}

#[test]
fn memory_enum_capture_updates_reject_dynamic_replacements_and_dead_later_targets() {
    use elaboration::verification::memory;
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Action=fn(*bool,*bool)->bool;impl Duplicate for Action {}
        enum Ops {left(Action),right(Action)} impl Duplicate for Ops {}
        fn first(a:*bool,b:*bool)->bool {a.*}
        struct Cell {value:bool} fn consume(value:Cell) {}";
    let package=compile(&format!("{prefix}
        fn inspect(left:*bool,right:*bool,next:Action)->bool {{var ops=Ops.left(first);
            let callback=|a:*bool,b:*bool| {{let result=ops match {{.left(action)=>action(a,b),.right(action)=>action(a,b)}};ops=Ops.right(next);result}};
            callback(left,right);callback(left,right)
        }}"));
    let inspect = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let error =
        memory::infer_in_package(&package, inspect.declaration, &[], &mut 300000).unwrap_err();
    assert!(
        error.contains("closure capture code exit has unknown environment identity"),
        "{error}"
    );
    for retire in [false, true] {
        let retired = if retire { "consume(right);" } else { "" };
        let package=compile(&format!("{prefix}
            fn run()->bool {{let left=true;let right=Cell {{value:false}};let pointer=right.value.ref;
                var ops=Ops.left(first);
                let callback=|a:*bool,b:*bool| {{let result=ops match {{.left(action)=>action(a,b),.right(action)=>action(a,b)}};ops=Ops.right(|a,b|b.*);result}};
                callback(left.ref,pointer);{retired}callback(left.ref,pointer)
            }}"));
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            600000,
        );
        if retire {
            assert!(result.unwrap_err().contains("unavailable"));
        } else {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(false));
        }
    }
}

#[test]
fn memory_returned_enum_capture_updates_keep_absence_and_future_identities() {
    use elaboration::verification::memory;
    let prefix="trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Action=fn(*bool,*bool)->bool;impl Duplicate for Action {}
        enum Ops {left(Action),right(Action)} impl Duplicate for Ops {}
        typealias Stateful=%{ops:Ops} fn(*bool,*bool)->bool;
        fn first(a:*bool,b:*bool)->bool {a.*} fn second(a:*bool,b:*bool)->bool {b.*}
        fn factory(again:bool)->Stateful {
            if again {return factory(false);}
            var ops=Ops.left(first);
            |a:*bool,b:*bool| {let result=ops match {.left(action)=>action(a,b),.right(action)=>action(a,b)};ops=Ops.right(second);result}
        }
        fn inspect(left:*bool,right:*bool)->bool {let callback=factory(true);callback(left,right);callback(left,right)}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let left=true;let right=false;inspect(left.ref,right.ref)}}"
    ));
    let function = |name: &str| package.functions.iter().find(|f| f.name == name).unwrap();
    let factory =
        memory::infer_in_package(&package, function("factory").declaration, &[], &mut 700000)
            .unwrap();
    assert!(
        factory.to_string().contains("(absent-variant)"),
        "{factory}"
    );
    let infer = |fuel: &mut usize| {
        memory::infer_in_package(&package, function("inspect").declaration, &[], fuel)
    };
    let summary = infer(&mut 700000).unwrap();
    assert_eq!(
        summary
            .accesses
            .iter()
            .filter(|a| a.read_value)
            .map(|a| a.target.parameter)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([0, 1]),
        "{summary}"
    );
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 700000).unwrap());
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
}

#[test]
fn memory_returned_enum_capture_updates_do_not_restore_pre_warmup_state() {
    for (warm, local) in [(false, false), (true, false), (true, true)] {
        let warmup = if local {
            "let x=true;let y=false;callback(x.ref,y.ref);"
        } else if warm {
            "callback(a,b);"
        } else {
            ""
        };
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            typealias Action=fn(*bool,*bool)->bool;impl Duplicate for Action {{}}
            enum Ops {{left(Action),right(Action)}} impl Duplicate for Ops {{}}
            typealias Stateful=%{{ops:Ops}} fn(*bool,*bool)->bool;
            fn first(a:*bool,b:*bool)->bool {{a.*}} fn second(a:*bool,b:*bool)->bool {{b.*}}
            fn factory(again:bool,a:*bool,b:*bool)->Stateful {{
                if again {{return factory(false,a,b);}}
                var ops=Ops.left(first);
                let callback=|a:*bool,b:*bool| {{let result=ops match {{.left(action)=>action(a,b),.right(action)=>action(a,b)}};ops=Ops.right(second);result}};
                {warmup}callback
            }}
            fn inspect(a:*bool,b:*bool)->bool {{let callback=factory(true,a,b);callback(a,b)}}
            fn run()->bool {{let a=true;let b=false;inspect(a.ref,b.ref)}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1200000
            )
            .unwrap_or_else(|error| {
                let summaries = package
                    .functions
                    .iter()
                    .filter(|f| ["factory", "inspect", "run"].contains(&f.name.as_str()))
                    .map(|f| {
                        (
                            f.name.clone(),
                            elaboration::verification::memory::infer_in_package(
                                &package,
                                f.declaration,
                                &[],
                                &mut 1200000,
                            )
                            .map(|summary| summary.to_string()),
                        )
                    })
                    .collect::<Vec<_>>();
                panic!("warm={warm}, local={local}: {error}; {summaries:?}")
            }),
            comptime_interpretor::Value::Bool(!warm)
        );
    }
}

#[test]
fn memory_fixed_callback_updates_abstract_plain_enum_capture_values() {
    use elaboration::verification::memory;
    for next in [false, true] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Mode {{left,right}} impl Duplicate for Mode {{}}
            fn inspect(a:*bool,b:*bool,next:bool)->bool {{
                let mode=Mode.left;var action=|a:*bool,b:*bool| mode match {{.left=>a.*,.right=>b.*}};
                let wrapper=|a:*bool,b:*bool,next:bool| {{
                    let result=action(a,b);let mode=if next {{Mode.left}} else {{Mode.right}};
                    action=|a:*bool,b:*bool| mode match {{.left=>a.*,.right=>b.*}};result
                }};
                wrapper(a,b,next);wrapper(a,b,next)
            }}
            fn run()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,{next})}}"));
        let inspect = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, inspect.declaration, &[], &mut 700000).unwrap();
        assert_eq!(
            summary
                .accesses
                .iter()
                .filter(|a| a.read_value)
                .map(|a| a.target.parameter)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([0, 1]),
            "{summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1000000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(next)
        );
    }
}

#[test]
fn local_storage_memory_summary_keeps_local_accesses_out_of_parameter_footprints() {
    use elaboration::verification::memory;
    let package = compile(
        "struct Pair {left:bool,right:bool}
        fn select(first:bool,a:*bool,b:*bool)->*bool {if first {a} else {b}}
        fn flip(pointer:*bool)->bool {pointer.* = not pointer.*;pointer.*}
        fn inspect(external:*bool,first:bool)->bool {
            let pair=Pair {left:false,right:true};
            let local=pair.left.ref;
            let pointer=select(first,local,external);
            let observed=flip(pointer);observed and pair.right
        }
        fn isolated()->bool {let value=false;let pointer=value.ref;flip(pointer)}",
    );
    let function = |name: &str| package.functions.iter().find(|f| f.name == name).unwrap();
    let infer = |name: &str, fuel: &mut usize| {
        memory::infer_in_package(&package, function(name).declaration, &[], fuel)
    };
    let summary = infer("inspect", &mut 200000).unwrap();
    assert!(!summary.accesses.is_empty(), "{summary}");
    assert!(
        summary.accesses.iter().all(|a| a.target.parameter == 0),
        "{summary}"
    );
    assert!(
        summary.accesses.iter().any(|a| a.write && a.read_value),
        "{summary}"
    );
    assert!(infer("isolated", &mut 200000).unwrap().accesses.is_empty());
    assert!(infer("inspect", &mut 1).is_err());
    assert_eq!(summary, infer("inspect", &mut 200000).unwrap());
}

#[test]
fn local_storage_memory_summary_rejects_escape_and_resolves_local_slots() {
    use elaboration::verification::memory;
    for source in [
        "fn subject()->*bool {let value=true;value.ref}",
        "fn identity(p:*bool)->*bool {p} fn subject()->*bool {let value=true;identity(value.ref)}",
        "fn subject(p:*bool,flag:bool)->*bool {let value=true;if flag {p} else {value.ref}}",
    ] {
        let package = compile(source);
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        assert!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 200000).is_err(),
            "{source}"
        );
    }
    let package = compile("fn subject(p:*bool)->bool {let slot=p.ref;slot.*.*}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 200000).unwrap();
    assert_eq!(summary.accesses.len(), 1, "{summary}");
    let access = &summary.accesses[0];
    assert_eq!(access.target.parameter, 0, "{summary}");
    assert!(
        access.target.input_fields.is_empty()
            && access.target.through.is_empty()
            && access.target.fields.is_empty(),
        "{summary}"
    );
    assert!(access.read_value && !access.write, "{summary}");
}

#[test]
fn local_storage_anonymous_fields_rebase_through_returned_pointer_helpers() {
    use elaboration::verification::memory;
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {flag:bool}
        fn keep(...properties:T)->T where T {properties}
        fn identity(p:*bool)->*bool {p}
        fn flip(p:*bool)->bool {p.* = not p.*;p.*}
        fn local(seed:bool)->bool {
            let fields=keep {first:Cell {flag:seed},second:Cell {flag:true}};
            let pointer=identity(fields.first.flag.ref);
            flip(pointer) and fields.second.flag
        }
        fn run()->bool {let value=true;let external=value.ref;local(false) and external.*}",
    );
    let local = package
        .functions
        .iter()
        .find(|f| f.name == "local")
        .unwrap();
    let summary = memory::infer_in_package(&package, local.declaration, &[], &mut 200000).unwrap();
    assert!(summary.accesses.is_empty(), "{summary}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn local_storage_defaults_read_prepared_pointers_in_the_original_storage_frame() {
    use elaboration::verification::memory;
    for (seed, moved) in [(false, false), (true, false), (true, true)] {
        let package = compile(
            &"trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {value:bool}
        fn read(p:*bool,.observed:bool=p.*)->bool {observed}
        fn local()->bool {let cell=Cell {value:SEED};let pointer=cell.value.ref;MOVE read(pointer)}
        fn run()->bool {let marker=true;let exposed=marker.ref;local() and exposed.*}"
                .replace("SEED", if seed { "true" } else { "false" })
                .replace("MOVE", if moved { "let consumed=cell;" } else { "" }),
        );
        let local = package
            .functions
            .iter()
            .find(|f| f.name == "local")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, local.declaration, &[], &mut 200000).unwrap();
        assert!(summary.accesses.is_empty(), "{summary}");
        assert!(memory::infer_in_package(&package, local.declaration, &[], &mut 1).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, local.declaration, &[], &mut 200000).unwrap()
        );
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1000000,
        );
        if moved {
            assert!(
                result
                    .err()
                    .expect("layout evidence cannot revive moved storage")
                    .to_string()
                    .contains("unavailable")
            );
        } else {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(seed));
        }
    }
}

#[test]
fn immediate_borrowed_closures_snapshot_origins_before_arguments_and_share_recursion() {
    use elaboration::verification::memory;
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn snapshot(a:*bool,b:*bool)->*bool {var p=a;(|ignored:bool| p)(do {p=b;true})}
        fn recursive(p:*bool,flag:bool)->bool {(|| if flag {p.*} else {recursive(p,true)})()}
        fn run()->bool {let a=true;let b=false;snapshot(a.ref,b.ref).* and recursive(a.ref,false)}",
    );
    for name in ["snapshot", "recursive"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 200000).unwrap();
        if name == "snapshot" {
            let targets = summary.result_targets.as_ref().unwrap();
            assert_eq!(targets.len(), 1, "{summary}");
            assert_eq!(targets.first().unwrap().parameter, 0, "{summary}");
        } else {
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.read_value && a.target.parameter == 0),
                "{summary}"
            );
        }
        assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err());
        assert_eq!(
            summary,
            memory::infer_in_package(&package, function.declaration, &[], &mut 200000).unwrap()
        );
    }
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn immediate_borrowed_closures_transport_known_code_arguments_and_captures() {
    use elaboration::verification::memory;
    for body in [
        "(|callback:fn(*bool)->bool| callback(p))(read)",
        "(|callback:fn(*bool)->bool| callback(p))(|q| q.*)",
        "do {let action=read;(|| action(p))()}",
        "do {let action=|q:*bool|q.*;(|| action(p))()}",
        "do {let flag=true;let action=|q:*bool|q.* and flag;(|| action(p))()}",
        "do {let flag=true;(|callback:%{flag:bool} fn(*bool)->bool|callback(p))(|q|q.* and flag)}",
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            fn read(p:*bool)->bool {{p.*}}
            fn subject(p:*bool)->bool {{{body}}}
            fn run()->bool {{let value=true;subject(value.ref)}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, function.declaration, &[], &mut 200000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.read_value && a.target.parameter == 0),
            "{body}: {summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1000000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{body}"
        );
    }
}

#[test]
fn immediate_borrowed_code_captures_keep_construction_identity_and_reject_unknowns() {
    use elaboration::verification::memory;
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn first(a:*bool,b:*bool)->*bool {a}
        fn second(a:*bool,b:*bool)->*bool {b}
        fn subject(a:*bool,b:*bool)->*bool {
            var action=first;
            (|ignored:bool|action(a,b))(do {action=second;true})
        }
        fn run()->bool {let a=true;let b=false;subject(a.ref,b.ref).*}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 200000).unwrap();
    let targets = summary.result_targets.as_ref().unwrap();
    assert_eq!(targets.len(), 1, "{summary}");
    assert_eq!(targets.first().unwrap().parameter, 0, "{summary}");
    assert!(memory::infer_in_package(&package, function.declaration, &[], &mut 1).is_err());
    assert_eq!(
        summary,
        memory::infer_in_package(&package, function.declaration, &[], &mut 200000).unwrap()
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for body in [
        "(||action(p))()",
        "(|callback:fn(*bool)->bool|callback(p))(action)",
    ] {
        let package = compile(&format!(
            "fn subject(p:*bool,action:fn(*bool)->bool)->bool {{{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        assert!(
            memory::infer_in_package(&package, function.declaration, &[], &mut 200000).is_err(),
            "{body}"
        );
    }
}

#[test]
fn immediate_borrowed_aggregate_results_keep_reads_and_callable_fields() {
    use elaboration::verification::memory;
    for body in [
        "(|| Packet {flag:p.*})().flag",
        "do {let (flag,other)=(|| (p.*,true))();flag and other}",
        "(|| Choice.some(p.*))() match {.some(flag)=>flag}",
        "(|| Ops {action:if p.* {read} else {invert}})().action(p)",
        "(|| if p.* {read} else {invert})()(p)",
        "(|| do {let observed=p.*;|q:*bool|observed and q.*})()(p)",
        "apply((|| if p.* {read} else {invert})(),p)",
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Packet {{flag:bool}} enum Choice {{some(bool)}}
            struct Ops {{action:fn(*bool)->bool}}
            fn read(p:*bool)->bool {{p.*}} fn invert(p:*bool)->bool {{not p.*}}
            fn apply(action:fn(*bool)->bool,p:*bool)->bool {{action(p)}}
            fn subject(p:*bool)->bool {{{body}}}
            fn run()->bool {{let value=true;subject(value.ref)}}"
        ));
        let subject = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        let summary =
            memory::infer_in_package(&package, subject.declaration, &[], &mut 200000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.read_value && a.target.parameter == 0),
            "{body}: {summary}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1000000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{body}"
        );
    }
}

#[test]
fn immediate_borrowed_results_retain_pointer_storage_and_reject_unknown_code() {
    use elaboration::verification::memory;
    let package =
        compile("struct Box {pointer:*bool} fn subject(p:*bool)->Box {(|| Box {pointer:p})()}");
    let subject = package
        .functions
        .iter()
        .find(|f| f.name == "subject")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, subject.declaration, &[], &mut 200000).unwrap();
    let targets = summary
        .pointer_fields
        .get(&vec![0])
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(targets.len(), 1);
    let target = targets.first().unwrap();
    assert_eq!(target.parameter, 0);
    assert!(
        target.input_fields.is_empty() && target.fields.is_empty() && target.through.is_empty()
    );
    for source in [
        "fn subject(p:*bool)->*bool {(|| do {let value=p.*;value.ref})()}",
        "fn subject(p:*bool,action:fn(*bool)->bool)->bool {(|| do {let read=p.*;action})()(p)}",
    ] {
        let package = compile(source);
        let subject = package
            .functions
            .iter()
            .find(|f| f.name == "subject")
            .unwrap();
        assert!(
            memory::infer_in_package(&package, subject.declaration, &[], &mut 200000).is_err(),
            "{source}"
        );
    }
}
#[test]
fn prepared_code_absence_requires_a_variant_guarded_callable_leaf() {
    use elaboration::verification::memory::{self, CallableInput, CodeResult};
    use std::collections::BTreeMap;
    let package = compile(
        "enum Choice {none,some(fn()->bool),scalar(bool)} fn forward(value:Choice)->Choice {value}",
    );
    let forward = package
        .functions
        .iter()
        .find(|function| function.name == "forward")
        .unwrap();
    let path = vec![usize::MAX, 1, 0];
    let inputs = BTreeMap::from([(
        CallableInput {
            parameter: 0,
            fields: path.clone(),
        },
        vec![],
    )]);
    let infer = |inputs: &memory::CodeInputs<'_>, fuel| {
        memory::infer_with_code_inputs(
            &package,
            forward.declaration,
            &[],
            &BTreeMap::new(),
            inputs,
            &mut { fuel },
        )
    };
    let summary = infer(&inputs, 100_000).unwrap();
    assert!(
        matches!(summary.code_fields.get(&path),Some(CodeResult::Known(candidates)) if candidates.is_empty())
    );
    let unknown = infer(&BTreeMap::new(), 100_000).unwrap();
    assert!(matches!(
        unknown.code_fields.get(&path),
        Some(CodeResult::Unknown)
    ));
    for fields in [
        vec![],
        vec![0],
        vec![usize::MAX, 2, 0],
        vec![usize::MAX, 99, 0],
        vec![usize::MAX, 1],
    ] {
        let invalid = BTreeMap::from([(
            CallableInput {
                parameter: 0,
                fields,
            },
            vec![],
        )]);
        assert!(infer(&invalid, 100_000).is_err());
    }
    assert!(infer(&inputs, 0).is_err());
    assert_eq!(summary, infer(&inputs, 100_000).unwrap());
}

#[test]
fn conditional_pointer_result_ranges_use_current_tags_and_pointee_values() {
    use elaboration::verification::{Limits, flow};
    for (initial, update, expected, proven) in [
        ("Choice.left(a.ref)", "", 7, true),
        ("Choice.right(b.ref)", "", 19, true),
        (
            "Choice.left(a.ref)",
            "choice=Choice.right(b.ref);",
            19,
            true,
        ),
        (
            "Choice.left(a.ref)",
            "choice=Choice.right(b.ref);",
            7,
            false,
        ),
        ("Choice.left(a.ref)", "a=23;", 23, true),
        ("Choice.left(a.ref)", "a=23;", 7, false),
        (
            "if flag {Choice.left(a.ref)} else {Choice.right(b.ref)}",
            "",
            7,
            false,
        ),
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            enum Choice {{left(*Byte),right(*Byte)}}
            fn pick(p:*Choice)->*Byte {{p.* match {{.left(x)=>x,.right(x)=>x}}}}
            fn relay(p:*Choice)->*Byte {{pick(p)}}
            fn run(flag:bool) {{var a:Byte=7;let b:Byte=19;var choice={initial};
                {update} let result=relay(choice.ref);assumes result.* == {expected};}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
        let check = report.points.last().unwrap().check_input.as_ref();
        assert_eq!(
            check.is_some_and(|check| check.prove(&mut Limits::default()).is_ok()),
            proven,
            "{initial}; {update}; expected {expected}: {report}"
        );
    }
}

#[test]
fn conditional_return_addresses_survive_retagging_but_not_stale_value_proofs() {
    use elaboration::verification::{Limits, flow};
    for (operation, expected, proven) in [
        ("choice=Choice.right(b.ref);", 7, true),
        ("choice=Choice.right(b.ref);", 19, false),
        ("result.* = 23;", 23, true),
        ("result.* = 23;", 7, false),
        ("a=23;", 23, true),
        ("a=23;", 7, false),
        ("change(result);", 7, false),
        ("choice=Choice.right(b.ref);change(result);", 7, false),
        ("choice=Choice.right(b.ref);b=23;", 7, true),
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            enum Choice {{left(*Byte),right(*Byte)}}
            fn pick(p:*Choice)->*Byte {{p.* match {{.left(x)=>x,.right(x)=>x}}}}
            fn change(p:*Byte) {{p.* = 23;}}
            fn run() {{var a:Byte=7;var b:Byte=19;var choice=Choice.left(a.ref);
                let result=pick(choice.ref);{operation}assumes result.* == {expected};}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
        assert_eq!(
            report
                .points
                .last()
                .unwrap()
                .check_input
                .as_ref()
                .is_some_and(|check| check.prove(&mut Limits::default()).is_ok()),
            proven,
            "{operation}; {expected}: {report}"
        );
    }
}

#[test]
fn conditional_return_ranges_compose_nested_payloads_and_loaded_roots() {
    use elaboration::verification::{Limits, flow};
    for (declarations, initializer, pick) in [
        (
            "enum Outer {left(Choice),right(Choice)}",
            "Outer.left(Choice.left(a.ref))",
            "p.* match {.left(c)=>c match {.left(x)=>x,.right(x)=>x},.right(c)=>c match {.left(x)=>x,.right(x)=>x}}",
        ),
        (
            "enum Outer {left(Choice),right(Choice)}",
            "Outer.right(Choice.right(b.ref))",
            "p.* match {.left(c)=>c match {.left(x)=>x,.right(x)=>x},.right(c)=>c match {.left(x)=>x,.right(x)=>x}}",
        ),
        (
            "enum Outer {left(*Choice),right(*Choice)}",
            "Outer.left(choice.ref)",
            "p.* match {.left(c)=>c.* match {.left(x)=>x,.right(x)=>x},.right(c)=>c.* match {.left(x)=>x,.right(x)=>x}}",
        ),
        (
            "struct Outer {choice:Choice}",
            "Outer {choice:Choice.left(a.ref)}",
            "p.choice.* match {.left(x)=>x,.right(x)=>x}",
        ),
    ] {
        let selected = if initializer.contains("b.ref") { 19 } else { 7 };
        for expected in [7, 19] {
            let package = compile(&format!(
                "typealias Byte=.u8'builtin;enum Choice {{left(*Byte),right(*Byte)}}
                {declarations} fn pick(p:*Outer)->*Byte {{{pick}}}
                fn relay(p:*Outer)->*Byte {{pick(p)}}
                fn run() {{let a:Byte=7;let b:Byte=19;let choice=Choice.left(a.ref);
                    let outer={initializer};let result=relay(outer.ref);assumes result.* == {expected};}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let report = flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
            assert_eq!(
                report
                    .points
                    .last()
                    .unwrap()
                    .check_input
                    .as_ref()
                    .is_some_and(|check| check.prove(&mut Limits::default()).is_ok()),
                expected == selected,
                "{initializer}; {expected}: {report}"
            );
        }
    }
}

#[test]
fn conditional_loop_writes_preserve_only_invariant_address_dependencies() {
    use elaboration::verification::{Limits, flow};
    for (body, proven) in [
        ("pick(choice.ref).* = 23;", true),
        ("relay(choice.ref).* = 23;", true),
        ("choice=Choice.right(b.ref);pick(choice.ref).* = 23;", false),
        ("pick(choice.ref).* = 23;choice=Choice.right(b.ref);", false),
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;enum Choice {{left(*Byte),right(*Byte)}}
            fn pick(p:*Choice)->*Byte {{p.* match {{.left(x)=>x,.right(x)=>x}}}}
            fn relay(p:*Choice)->*Byte {{pick(p)}}
            fn run(flag:bool) {{var a:Byte=7;var b:Byte=19;let other=b.ref;
                var choice=Choice.left(a.ref);var again=true;
                while flag {{{body} if not again {{break;}} again=false;}}
                assumes other.* == 19;}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
        assert_eq!(
            report
                .points
                .last()
                .unwrap()
                .check_input
                .as_ref()
                .is_some_and(|check| check.prove(&mut Limits::default()).is_ok()),
            proven,
            "{body}: {report}"
        );
    }
}

#[test]
fn conditional_scalar_writers_and_default_factories_preserve_loop_footprints() {
    use elaboration::verification::{Limits, flow};
    for (body, proven) in [
        ("write_choice(choice.ref);", true),
        ("write_choice_return(choice.ref).* = 24;", true),
        ("wrap(choice.ref).target.* = 24;", true),
        ("write_default(choice.ref);", true),
        ("pick_handle(handle.ref).* = 24;", true),
        (
            "choice=Choice.right(b.ref);pick_handle(handle.ref).* = 24;",
            false,
        ),
        (
            "handle=Handle.some(other_choice.ref);pick_handle(handle.ref).* = 24;",
            false,
        ),
        (
            "choice=Choice.right(b.ref);write_choice(choice.ref);",
            false,
        ),
        (
            "write_choice(choice.ref);choice=Choice.right(b.ref);",
            false,
        ),
        (
            "choice=Choice.right(b.ref);write_default(choice.ref);",
            false,
        ),
    ] {
        let package = compile_in_with_resources(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}} typealias Byte=.u8'builtin;enum Choice {{left(*Byte),right(*Byte)}}
            impl Duplicate for Choice {{}}
            struct Link {{target:*Byte}} enum Handle {{some(*Choice)}} impl Duplicate for Handle {{}}
            fn pick(p:*Choice)->*Byte {{p.* match {{.left(x)=>x,.right(x)=>x}}}}
            fn pick_handle(h:*Handle)->*Byte {{h.* match {{.some(p)=>pick(p)}}}}
            fn wrap(p:*Choice)->Link {{Link {{target:pick(p)}}}}
            fn write_choice(p:*Choice) {{pick(p).* = 23;}}
            fn write_choice_return(p:*Choice)->*Byte {{let result=pick(p);result.* = 23;result}}
            fn write_default(p:*Choice,.target:*Byte=pick(p)) {{target.* = 23;}}
            fn run(flag:bool) {{var a:Byte=7;var b:Byte=19;let other=b.ref;
                var choice=Choice.left(a.ref);let other_choice=Choice.right(b.ref);
                var handle=Handle.some(choice.ref);var again=true;
                while flag {{{body} if not again {{break;}} again=false;}}
                assumes other.* == 19;}} fn execute() {{run(true);}}"
        ),0,Some("Duplicate"),Some("Retire"));
        let executed = luna::execution::interpret_with_protocols(
            &package,
            "execute",
            "Duplicate",
            "Retire",
            1_000_000,
        );
        if proven {
            assert!(executed.is_ok(), "{body}: {executed:?}");
        } else {
            assert!(
                executed.unwrap_err().contains("ProgramContractRejected"),
                "{body}: expected a failing runtime check"
            );
        }
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let report = flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
        assert_eq!(
            report
                .points
                .last()
                .unwrap()
                .check_input
                .as_ref()
                .is_some_and(|check| check.prove(&mut Limits::default()).is_ok()),
            proven,
            "{body}: {report}"
        );
    }
}

#[test]
fn conditional_loop_dependencies_exclude_unrelated_sibling_fields() {
    use elaboration::verification::{Limits, flow};
    for (field_ty, initial, replacement, read) in [
        (
            "Choice",
            "Choice.left(a.ref)",
            "Choice.right(b.ref)",
            "p.choice.*",
        ),
        ("*Choice", "choice.ref", "other_choice.ref", "p.choice.*.*"),
    ] {
        for (body, proven) in [
            ("holder.count=24;pick(holder.ref).* = 23;".to_owned(), true),
            (
                "touch(holder.ref);pick(holder.ref).* = 23;".to_owned(),
                true,
            ),
            ("pick(holder.ref).* = 23;holder.count=24;".to_owned(), true),
            (
                format!("holder.choice={replacement};pick(holder.ref).* = 23;"),
                false,
            ),
            (
                format!("pick(holder.ref).* = 23;holder.choice={replacement};"),
                false,
            ),
            (
                format!("holder=Holder {{choice:{replacement},count:24}};pick(holder.ref).* = 23;"),
                false,
            ),
        ] {
            let package = compile_in_with_resources(
                &format!(
                    "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                typealias Byte=.u8'builtin;enum Choice {{left(*Byte),right(*Byte)}}
                impl Duplicate for Choice {{}}
                struct Holder {{choice:{field_ty},count:Byte}}
                fn pick(p:*Holder)->*Byte {{{read} match {{.left(x)=>x,.right(x)=>x}}}}
                fn touch(p:*Holder) {{p.count.* = 24;}}
                fn run(flag:bool) {{var a:Byte=7;var b:Byte=19;let other=b.ref;
                    var choice=Choice.left(a.ref);let other_choice=Choice.right(b.ref);
                    var holder=Holder {{choice:{initial},count:0}};var again=true;
                    while flag {{{body} if not again {{break;}} again=false;}}
                    assumes other.* == 19;}}
                fn execute() {{run(true);}}"
                ),
                0,
                Some("Duplicate"),
                Some("Retire"),
            );
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let report = flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
            assert_eq!(
                report
                    .points
                    .last()
                    .unwrap()
                    .check_input
                    .as_ref()
                    .is_some_and(|check| check.prove(&mut Limits::default()).is_ok()),
                proven,
                "{field_ty}; {body}: {report}"
            );
            let executed = luna::execution::interpret_with_protocols(
                &package,
                "execute",
                "Duplicate",
                "Retire",
                1_000_000,
            );
            if proven {
                assert!(executed.is_ok(), "{body}: {executed:?}");
            } else {
                let error = executed.unwrap_err();
                assert!(error.contains("ProgramContractRejected"), "{body}: {error}");
            }
        }
    }
}

#[test]
fn tuple_pointer_slots_cross_calls_without_losing_snapshot_or_write_identity() {
    use elaboration::verification::{Limits, flow};
    for (ty, pattern, first, second) in [
        ("Tuple<Byte,*Byte>", "(_,target)", "(0,a.ref)", "(1,b.ref)"),
        (
            "Tuple<Tuple<Byte,*Byte>,bool>",
            "((_,target),_)",
            "((0,a.ref),true)",
            "((1,b.ref),false)",
        ),
    ] {
        for record in [false, true] {
            for expected in [23, 7] {
                let (parameter, read, initial, replacement, address) = if record {
                    (
                        "*Holder".to_owned(),
                        "p.payload.*".to_owned(),
                        format!("Holder {{payload:{first}}}"),
                        format!("payload.payload={second};"),
                        "payload.ref",
                    )
                } else {
                    (
                        format!("*({ty})"),
                        "p.*".to_owned(),
                        first.to_owned(),
                        format!("payload={second};"),
                        "payload.ref",
                    )
                };
                let binding_ty = if record { "Holder" } else { ty };
                let package = tuple_support::compile_with_tuple_selectors(&format!(
                    "trait Duplicate {{}} trait Retire {{fn drop(*self);}} typealias Byte=.u8'builtin;
                    impl Duplicate for Tuple<A,B> where A, B {{}}
                    struct Holder {{payload:{ty}}}
                    fn pick(p:{parameter})->*Byte {{let {pattern}={read};target}}
                    fn relay(p:{parameter})->*Byte {{pick(p)}}
                    fn run() {{var a:Byte=7;var b:Byte=19;var payload:{binding_ty} = {initial};
                        let saved=relay({address});{replacement}
                        let current=relay({address});current.* = 23;
                        assumes saved.* == 7;assumes current.* == {expected};}}"
                ), &Default::default(), luna::protocols::type_constructors::Selectors { copy:Some("Duplicate"), drop:Some("Retire"), ..Default::default() }).unwrap();
                let run = package.functions.iter().find(|f| f.name == "run").unwrap();
                let report =
                    flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
                for (check, proven) in report.points.iter().zip([true, expected == 23]) {
                    assert_eq!(
                        check
                            .check_input
                            .as_ref()
                            .is_some_and(|check| check.prove(&mut Limits::default()).is_ok()),
                        proven,
                        "{ty}; record={record}; expected={expected}: {report}"
                    );
                }
                assert_eq!(report.points.len(), 2, "{report}");
                let result = luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1_000_000,
                );
                if expected == 23 {
                    assert!(result.is_ok(), "{result:?}");
                } else {
                    let error = result.unwrap_err();
                    assert!(error.contains("ProgramContractRejected"), "{error}");
                }
            }
        }
    }
}

#[test]
fn completed_aggregate_constants_supply_field_facts_without_sharing_mutable_storage() {
    use elaboration::verification::{Limits, flow};
    for (declaration, ty, initial, extract, replace, correct) in [
        (
            "",
            "Tuple<Byte,Byte>",
            "(7,19)",
            "let (left,right)=value;",
            "value=(23,29);",
            "left==7 and right==19",
        ),
        (
            "",
            "Tuple<Tuple<Byte,bool>,Byte>",
            "((7,true),19)",
            "let ((left,flag),right)=value;",
            "value=((23,false),29);",
            "left==7 and flag and right==19",
        ),
        (
            "struct Pair {left:Byte,right:Byte}",
            "Pair",
            "Pair {left:7,right:19}",
            "let left=value.left;let right=value.right;",
            "value.left=23;",
            "left==7 and right==19",
        ),
        (
            "enum Choice {none,pair(Byte,Byte)}",
            "Choice",
            "Choice.pair(7,19)",
            "let (left,right):Tuple<Byte,Byte> = value match {.none=>(0,0),.pair(a,b)=>(a,b)};",
            "value=Choice.none;",
            "left==7 and right==19",
        ),
    ] {
        for truth in [true, false] {
            let condition = if truth {
                correct.to_owned()
            } else {
                format!("not ({correct})")
            };
            let package = tuple_support::compile_with_tuple(&format!(
                "typealias Byte=.u8'builtin;{declaration} const seed:{ty} = {initial};
                comptime fn run() {{var value:{ty} = seed;{extract}{replace}assumes {condition};}}"
            ));
            let run = package.functions.iter().find(|f| f.name == "run").unwrap();
            let report = flow::analyze_in_package(&package, run, &mut Limits::default()).unwrap();
            let check = report.points.last().unwrap().check_input.as_ref();
            assert_eq!(
                check.is_some_and(|c| c.prove(&mut Limits::default()).is_ok()),
                truth,
                "{ty}; {condition}: {report}"
            );
            let result = luna::invocation::invoke(&package, "run", 1_000_000);
            if truth {
                assert!(result.is_ok(), "{result:?}");
            } else {
                let error = result.unwrap_err();
                assert!(error.contains("ProgramContractRejected"), "{error}");
            }
        }
    }
}

#[test]
fn monotone_cubic_correlations_prove_unsigned_subtraction_safety() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn cubic(value:Byte)->Byte where requires (value >= 1 and value <= 6) {
            value * value * value - value
        }
        fn run()->Byte {cubic(6)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(210)
    );
}

#[test]
fn borrowed_string_views_do_not_gain_pointer_free_layout_from_copy() {
    for (extra, body) in [
        ("fn inspect(value:str)->bool {true}", "inspect(\"static\")"),
        (
            "struct Text {view:str} fn inspect(value:Text)->bool {true}",
            "inspect(Text {view:\"static\"})",
        ),
        (
            "fn inspect(value:Tuple<bool,str>)->bool {true}",
            "inspect((true,\"static\"))",
        ),
    ] {
        let package = tuple_support::compile_with_tuple(&format!(
            "typealias str=.str'builtin; trait Duplicate {{}} trait Retire {{fn drop(*self);}} impl Duplicate for str {{}} {extra} fn entry()->bool {{{body}}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(error.contains("string-view memory provenance"), "{error}");
        assert!(error.contains("backing character storage"), "{error}");
    }
}

#[test]
fn handler_lexical_data_return_keeps_parameter_accesses_separate_from_completion() {
    use elaboration::verification::memory;
    for result in ["bool", "Byte"] {
        for value in ["value.*", "do {value.* = true; value.*}"] {
            let package = compile(&format!(
                "typealias Byte = .u8'builtin;
                 effect inspect(pointer:*bool)->{result};
                 fn subject(pointer:*bool)->#inspect {result} {{inspect(pointer)#}}
                 fn handled(pointer:*bool)->bool {{
                    subject(pointer)# {{inspect(value)=>do {{value.* = true;return {value};}},}};
                    false
                 }}"
            ));
            let function = package
                .functions
                .iter()
                .find(|f| f.name == "handled")
                .unwrap();
            let infer = |fuel: &mut usize| {
                memory::infer_in_package(&package, function.declaration, &[], fuel)
            };
            let summary = infer(&mut 100000).unwrap();
            assert!(summary.effect_inputs.is_empty(), "{summary}");
            assert!(summary.result_targets.is_none(), "{summary}");
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 0 && a.read_value),
                "{summary}"
            );
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 0 && a.write),
                "{summary}"
            );
            assert!(infer(&mut 1).is_err());
            assert_eq!(summary, infer(&mut 100000).unwrap());
        }
    }
}

#[test]
fn handler_local_closure_accesses_are_charged_at_invocation() {
    use elaboration::verification::memory;
    for (body, invoked) in [
        (
            "let change=|| {value.* = true; return value.*;}; return change();",
            true,
        ),
        (
            "let change=|| {value.* = true; return value.*;}; return false;",
            false,
        ),
        (
            "let borrow=|| {return value;}; let local=borrow(); local.* = true; return local.*;",
            true,
        ),
    ] {
        let package = compile(&format!(
            "effect inspect(pointer:*bool)->bool;
             fn subject(pointer:*bool)->#inspect bool {{inspect(pointer)#}}
             fn handled(pointer:*bool)->bool {{
                subject(pointer)# {{inspect(value)=>do {{{body}}},}};
                false
             }}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "handled")
            .unwrap();
        let mut handler = None;
        nir::visit::expression(&function.body, &mut 10000, |node, _| {
            if let nir::program::ExprKind::EffectHandle { handlers, .. } = &node.kind {
                handler = Some(handlers[0].clone());
            }
            Ok(())
        })
        .unwrap();
        let handler = handler.unwrap();
        let infer = |fuel: &mut usize| {
            memory::infer_handler_arm_memory(&package, &handler, &handler.arms[0], fuel)
        };
        let summary = infer(&mut 100000).unwrap();
        assert!(summary.memory.result_targets.is_none(), "{summary}");
        assert_eq!(
            summary
                .accesses
                .iter()
                .any(|a| a.parameter == 0 && a.read_value),
            invoked,
            "{summary}"
        );
        assert_eq!(
            summary.accesses.iter().any(|a| a.parameter == 0 && a.write),
            invoked,
            "{summary}"
        );
        assert!(infer(&mut 1).is_err());
        assert_eq!(summary, infer(&mut 100000).unwrap());
    }
}

#[test]
fn nested_handler_resume_retains_following_parameter_accesses() {
    use elaboration::verification::memory;
    for exit in ["false", "resume false;"] {
        let package = compile(&format!(
            "effect inspect(first:*bool,second:*bool)->bool;
             effect change(pointer:*bool)->bool;
             fn handled(first:*bool,second:*bool)->bool {{
                inspect(first,second)# {{inspect(a,b)=>do {{
                    change(a)# {{change(target)=>do {{target.* = true; {exit}}},}};
                    b.* = true;
                    return a.* and b.*;
                }},}};
                false
             }}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "handled")
            .unwrap();
        let infer =
            |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
        let summary = infer(&mut 100000).unwrap();
        for parameter in [0, 1] {
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == parameter && a.write),
                "{summary}"
            );
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == parameter && a.read_value),
                "{summary}"
            );
        }
        assert!(summary.effect_inputs.is_empty(), "{summary}");
        assert!(infer(&mut 1).is_err());
        assert_eq!(summary, infer(&mut 100000).unwrap());
    }
}

#[test]
fn handler_memory_captures_complete_stable_callback_evidence() {
    use elaboration::verification::memory;
    for callback in [
        "change",
        "|value:*bool| {value.* = true;}",
        "|value:*bool| {value.* = flag;}",
        "if flag {change} else {other}",
    ] {
        for invocation in [
            "callback(value)",
            "relay(value)# {relay(target)=>do {callback(target);false},}",
        ] {
            let package = compile(&format!(
                "effect inspect(pointer:*bool)->bool;
             effect relay(pointer:*bool)->bool;
             fn subject(pointer:*bool)->#inspect bool {{inspect(pointer)#}}
             fn change(value:*bool) {{value.* = true;}}
             fn other(value:*bool) {{value.* = false;}}
             fn handled(pointer:*bool,flag:bool)->bool {{
                let callback={callback};
                subject(pointer)# {{inspect(value)=>do {{{invocation};return value.*;}},}};
                false
             }}"
            ));
            let function = package
                .functions
                .iter()
                .find(|f| f.name == "handled")
                .unwrap();
            let infer = |fuel: &mut usize| {
                memory::infer_in_package(&package, function.declaration, &[], fuel)
            };
            let summary = infer(&mut 100000).unwrap();
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 0 && a.write),
                "{summary}"
            );
            assert!(
                summary
                    .accesses
                    .iter()
                    .any(|a| a.target.parameter == 0 && a.read_value),
                "{summary}"
            );
            assert!(summary.effect_inputs.is_empty(), "{summary}");
            assert!(infer(&mut 1).is_err());
            assert_eq!(summary, infer(&mut 100000).unwrap());
        }
    }
}

#[test]
fn handler_memory_callback_snapshot_rejects_mutation_and_address_exposure() {
    use elaboration::verification::memory;
    for (before, subject, body) in [
        (
            "",
            "inspect(pointer)#",
            "callback=other; callback(value); false",
        ),
        (
            "let alias=callback.ref;",
            "inspect(pointer)#",
            "callback(value); false",
        ),
        (
            "",
            "(do {callback=other;inspect(pointer)#})#",
            "callback(value); false",
        ),
    ] {
        let package = compile(&format!(
            "effect inspect(pointer:*bool)->bool;
             fn change(value:*bool) {{value.* = true;}}
             fn other(value:*bool) {{value.* = false;}}
             fn handled(pointer:*bool)->bool {{
                var callback=change;
                {before}
                {subject} {{inspect(value)=>do {{{body}}},}}
             }}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "handled")
            .unwrap();
        let error =
            memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap_err();
        assert!(
            error.contains("rebinding") || error.contains("addressing captured"),
            "{error}"
        );
    }
}

#[test]
fn handler_memory_does_not_invent_opaque_callback_behavior() {
    use elaboration::verification::memory;
    for source in [
        "effect inspect(pointer:*bool)->bool;
        fn handled(pointer:*bool, callback:fn(*bool)->bool)->bool {
            inspect(pointer)# {inspect(value)=>callback(value),}
        }",
        "typealias Action=fn(*bool)->bool;enum Choice {none,some(Action)}
         effect inspect(pointer:*bool)->bool;
         fn handled(pointer:*bool,callback:Choice)->bool {
             inspect(pointer)# {inspect(value)=>callback match {.none=>false,.some(call)=>call(value),},}
         }",
    ] { let package = compile(source);
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "handled")
        .unwrap();
    let error =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap_err();
    assert!(
        error.contains("call") || error.contains("provenance"),
        "{error}"
    );
    }
}

#[test]
fn handler_memory_callback_candidates_union_distinct_parameter_writes() {
    use elaboration::verification::memory;
    let package = compile(
        "effect inspect(first:*bool,second:*bool)->bool;
        fn left(first:*bool,second:*bool) {first.* = true;}
        fn right(first:*bool,second:*bool) {second.* = true;}
        fn handled(first:*bool,second:*bool,flag:bool)->bool {
            let callback=if flag {left} else {right};
            inspect(first,second)# {inspect(a,b)=>do {callback(a,b);false},}
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "handled")
        .unwrap();
    let summary =
        memory::infer_in_package(&package, function.declaration, &[], &mut 100000).unwrap();
    for parameter in [0, 1] {
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.target.parameter == parameter && a.write),
            "{summary}"
        );
    }
}

#[test]
fn handler_memory_snapshot_uses_candidate_at_installation() {
    use elaboration::verification::memory;
    let package = compile(
        "effect inspect(first:*bool,second:*bool)->bool;
        fn left(first:*bool,second:*bool) {first.* = true;}
        fn right(first:*bool,second:*bool) {second.* = true;}
        fn handled(first:*bool,second:*bool)->bool {
            var callback=left;
            callback=right;
            let result=inspect(first,second)# {inspect(a,b)=>do {callback(a,b);false},};
            callback=left;
            result
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "handled")
        .unwrap();
    let infer =
        |fuel: &mut usize| memory::infer_in_package(&package, function.declaration, &[], fuel);
    let summary = infer(&mut 100000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 1 && a.write),
        "{summary}"
    );
    assert!(
        !summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.write),
        "{summary}"
    );
    assert!(infer(&mut 1).is_err());
    assert_eq!(summary, infer(&mut 100000).unwrap());
}
