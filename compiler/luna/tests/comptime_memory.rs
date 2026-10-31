//! Transient comptime storage must not escape constant materialization.
#[path = "support/execution.rs"]
mod execution_support;
use comptime_interpretor::{Budget, Interpreter, Value};
use execution_support::{compile, compile_with_protocols, id};

#[test]
fn comptime_enum_pointer_copy_checks_inactive_variants_and_nominal_wrappers() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {}trait Retire {fn drop(*self);}
        struct Payload {flag:bool}impl Duplicate for Payload {}
        enum Choice {empty,item(Payload)}impl Duplicate for Choice {}
        newtype Wrapped=Choice;impl Duplicate for Wrapped {}
        comptime fn compute()->bool {
            let value=Choice.empty.as(Wrapped);
            let saved=value.ref.*;
            value.ref.* = Choice.item(Payload {flag:true}).as(Wrapped);
            let original=saved.as(Choice) match {.empty=>true,.item(_)=>false};
            let updated=value.as(Choice) match {.empty=>false,.item(p)=>p.flag};
            original and updated
        }
        const VALUE=compute();comptime fn result()->bool {VALUE}
        comptime fn Result()->Type {if compute() {bool} else {.u8'builtin}}
        typealias Chosen=Result();fn run()->Chosen {true}";
    let compile = |source: &str| {
        execution_support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
    };
    let package = compile(source).unwrap();
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "result"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    // Only the empty variant is ever constructed in these rejecting programs.
    let empty = source
        .replace(
            "value.ref.* = Choice.item(Payload {flag:true}).as(Wrapped);",
            "value.ref.* = Choice.empty.as(Wrapped);",
        )
        .replace("original and updated", "original and not updated");
    for rejected in [
        empty.replace("impl Duplicate for Payload {}", ""),
        empty.replace("impl Duplicate for Choice {}", ""),
        empty.replace("impl Duplicate for Wrapped {}", ""),
        format!("{empty}impl Retire for Payload {{fn drop(*self) {{}}}}"),
    ] {
        let failure = compile(&rejected).unwrap_err();
        assert!(failure.contains("pointer Copy evidence"), "{failure}");
    }
}

#[test]
fn comptime_aggregate_pointer_copy_preserves_external_storage_identity() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}impl Duplicate for Link {}
        comptime fn compute()->bool {
            let flag=false;let link=Link {target:flag.ref};
            let copied=link.ref.*;copied.target.* = true;
            flag and link.target.*
        }
        const VALUE=compute();comptime fn result()->bool {VALUE}
        comptime fn Result()->Type {if compute() {bool} else {.u8'builtin}}
        typealias Chosen=Result();fn run()->Chosen {true}";
    let compile = |source: &str| {
        execution_support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
    };
    let package = compile(source).unwrap();
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "result"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let escaped = format!(
        "{source}const ESCAPED=do {{let flag=true;let link=Link {{target:flag.ref}};link.ref.*}};"
    );
    assert!(
        compile(&escaped).is_err(),
        "aggregate pointer escaped constant materialization"
    );
    let expired = source.replace(
        "let flag=false;let link=Link {target:flag.ref};",
        "let flag=false;let link=do {let temporary=false;Link {target:temporary.ref}};",
    );
    let failure = compile(&expired).unwrap_err();
    assert!(failure.contains("ExpiredPointer"), "{failure}");
}

#[test]
fn comptime_generic_pointer_copy_checks_instantiated_resources() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {}trait Retire {fn drop(*self);}
        struct Leaf {flag:bool}impl Duplicate for Leaf {}
        struct Cell where T {value:T}
        impl Duplicate for Cell<T> where T :- Duplicate {}
        comptime fn read(p:*T)->T where T :- Duplicate {p.*}
        comptime fn write(p:*T,value:T) where T :- Duplicate {p.* = value;}
        comptime fn compute()->bool {
            let cell=Cell<Leaf> {value:Leaf {flag:true}};
            let copied=read(cell.ref);
            write(cell.ref,Cell<Leaf> {value:Leaf {flag:false}});
            copied.value.flag and not cell.value.flag
        }
        const VALUE=compute();comptime fn result()->bool {VALUE}
        comptime fn Result()->Type {if compute() {bool} else {.u8'builtin}}
        typealias Chosen=Result();fn run()->Chosen {true}";
    let compile = |source: &str| {
        execution_support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
    };
    let package = compile(source).unwrap();
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "result"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let invalid = format!("{source}impl Retire for Leaf {{fn drop(*self) {{}}}}");
    let failure = compile(&invalid).unwrap_err();
    assert!(failure.contains("pointer Copy evidence"), "{failure}");
    // An unconstrained helper can be formed, but cannot bypass the concrete
    // invocation's permission check merely by hiding the operand behind T.
    let unchecked = source
        .replace("where T :- Duplicate {p.*}", "where T {p.*}")
        .replace(
            "where T :- Duplicate {p.* = value;}",
            "where T {p.* = value;}",
        )
        .replace("impl Duplicate for Cell<T> where T :- Duplicate {}", "");
    let failure = compile(&unchecked).unwrap_err();
    assert!(failure.contains("no Copy capability"), "{failure}");
}

#[test]
fn comptime_copy_permissions_keep_nominal_identity_across_branches() {
    let source = "typealias Type=.Type'builtin;trait Duplicate {}trait Retire {fn drop(*self);}
        struct Allowed {flag:bool}struct Other {flag:bool}impl Duplicate for Allowed {}
        comptime fn Choose(flag:bool)->Type {
            if flag {let value=Allowed {flag:true};let copied=value.ref.*;bool}
            else {let value=Other {flag:true};let copied=value.ref.*;bool}
        }
        typealias Result=Choose(true);fn run()->Result {true}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let failure = execution_support::compile_with_source_bindings(
        &format!("{source}typealias Rejected=Choose(false);"),
        &Default::default(),
        selectors,
    )
    .unwrap_err();
    assert!(failure.contains("no Copy capability"), "{failure}");
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let completed = format!(
        "{source}impl Duplicate for Other {{}}typealias Second=Choose(false);fn second()->Second {{false}}"
    );
    let package =
        execution_support::compile_with_source_bindings(&completed, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "second", 100_000).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn comptime_aggregate_pointer_transfers_require_copy_and_cleanup_evidence() {
    let source = "typealias Type=.Type'builtin;trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {flag:bool} impl Duplicate for Cell {}
        comptime fn compute()->bool {
            let cell=Cell {flag:true};let copied=cell.ref.*;
            cell.ref.* = Cell {flag:false};copied.flag and not cell.flag
        }
        const VALUE=compute();comptime fn read()->bool {VALUE}
        comptime fn Result()->Type {if compute() {bool} else {.u8'builtin}}
        typealias Chosen=Result();fn run()->Chosen {true}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "read"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("impl Duplicate for Cell {}", ""),
        format!("{source}impl Retire for Cell {{fn drop(*self) {{}}}}"),
    ] {
        let selectors = luna::protocols::type_constructors::Selectors {
            copy: Some("Duplicate"),
            drop: Some("Retire"),
            ..Default::default()
        };
        assert!(
            execution_support::compile_with_source_bindings(
                &invalid,
                &Default::default(),
                selectors
            )
            .is_err()
        );
    }
    let unbound = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        unbound.contains("requires bound Copy and Drop protocols"),
        "{unbound}"
    );
    // Exercise both the ordinary-value path and the Type scheduler failure path.
    for input in [
        source.to_string(),
        source.replace("const VALUE=compute();comptime fn read()->bool {VALUE}", ""),
    ] {
        let missing = input.replace("impl Duplicate for Cell {}", "");
        let selectors = luna::protocols::type_constructors::Selectors {
            copy: Some("Duplicate"),
            drop: Some("Retire"),
            ..Default::default()
        };
        let failure = execution_support::compile_with_source_bindings(
            &missing,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(
            failure.contains("pointer Copy evidence") && failure.contains("no Copy capability"),
            "{failure}"
        );
    }
    let copy_only = luna::protocols::type_constructors::Selectors {
        copy: Some("Duplicate"),
        ..Default::default()
    };
    assert!(
        execution_support::compile_with_source_bindings(source, &Default::default(), copy_only)
            .is_err()
    );
    let nested = source
        .replace(
            "struct Cell {flag:bool}",
            "struct Leaf {flag:bool}impl Duplicate for Leaf {}struct Cell {leaf:Leaf}",
        )
        .replace("Cell {flag:true}", "Cell {leaf:Leaf {flag:true}}")
        .replace("Cell {flag:false}", "Cell {leaf:Leaf {flag:false}}")
        .replace("copied.flag", "copied.leaf.flag")
        .replace("cell.flag", "cell.leaf.flag");
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(&nested, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        nested.replace("impl Duplicate for Leaf {}", ""),
        format!("{nested}impl Retire for Leaf {{fn drop(*self) {{}}}}"),
    ] {
        let selectors = luna::protocols::type_constructors::Selectors {
            copy: Some("Duplicate"),
            drop: Some("Retire"),
            ..Default::default()
        };
        assert!(
            execution_support::compile_with_source_bindings(
                &invalid,
                &Default::default(),
                selectors
            )
            .is_err()
        );
    }
}

#[test]
fn comptime_bit_pointer_writes_check_lifetimes_bounds_and_control_flow() {
    let source = "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;
        comptime fn payload()->.BitVec'builtin<1> {1}
        comptime fn expired()->*Word {let word:Word=0;word.ref}
        comptime fn bad() {let p=expired();p.*'bitvec[0]=payload();}
        comptime fn early()->Word {let word:Word=0;word.ref.*'bitvec[do {return 7;}]=payload();word}
        comptime fn edit(p:*Word,i:Byte) {assumes i < 16;p.*'bitvec[i]=1;}
        comptime fn good()->Word {let word:Word=0;edit(word.ref,4);word}";
    let package = compile(source);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "bad"), vec![]).unwrap_err().kind,
        comptime_interpretor::ErrorKind::ExpiredPointer
    );
    assert_eq!(
        execution_support::calls(&package, &vm),
        ["bad", "expired", "payload"]
    );
    assert_eq!(
        vm.invoke(id(&package, "good"), vec![]).unwrap(),
        Value::UnsignedInteger(16)
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "early"), vec![]).unwrap(),
        Value::UnsignedInteger(7)
    );
    assert_eq!(execution_support::calls(&package, &vm), ["early"]);
    for body in [
        "comptime fn rejected(p:*Word,i:Byte) {p.*'bitvec[i]=1;} const BAD=do {let word:Word=0;rejected(word.ref,16);word};",
        "comptime fn rejected(p:*Word,i:Byte) {assumes i <= 16;p.*'bitvec[i]=1;} const BAD=do {let word:Word=0;rejected(word.ref,16);word};",
        "comptime fn rejected(p:*bool) {p.*'bitvec[0]=1;}",
        "comptime fn rejected(p:*Word) {p.*'bitvec[0..4]=16;}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{body}"), &Default::default()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn comptime_bit_pointer_writes_preserve_order_and_rhs_updates() {
    let source="typealias Word=.u16'builtin;typealias Byte=.u8'builtin;typealias Payload=.BitVec'builtin<4>;
        comptime fn target(p:*Word)->*Word {p}
        comptime fn low(trace:*bool)->result:Byte where ensures result == 4 {trace.* = true;4}
        comptime fn high(trace:*bool)->result:Byte where ensures result == 8 {trace.* = not trace.*;8}
        comptime fn payload(p:*Word,trace:*bool)->Payload {p.* = if trace.* {0} else {0xABCD};0b1010}
        comptime fn run()->Word {var word:Word=0x1234;let trace=false;target(word.ref).*'bitvec[low(trace.ref)..high(trace.ref)]=payload(word.ref,trace.ref);word}
        comptime fn rebind()->Word {var first:Word=0x1234;var second:Word=0xFFFF;var p=first.ref;
            let replacement:Payload=0b1010;p.*'bitvec[do {p=second.ref;let i:Byte=4;i}..8]=replacement;first}
        comptime fn empty()->Word {var word:Word=0;let p=word.ref;let i:Byte=16;p.*'bitvec[i..i]=do {p.* = 0xABCD;0};word}
        const RESULT=run();comptime fn frozen()->Word {RESULT}";
    let package = compile(source);
    for (entry, expected) in [
        ("run", 0xABAD),
        ("rebind", 0x12A4),
        ("empty", 0xABCD),
        ("frozen", 0xABAD),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, entry), vec![]).unwrap(),
            Value::UnsignedInteger(expected)
        );
        if entry == "run" {
            assert_eq!(
                execution_support::calls(&package, &vm),
                ["run", "target", "low", "high", "payload"]
            );
        }
    }
}

#[test]
fn scalar_comptime_pointers_preserve_aliases_and_call_writes() {
    let source = "struct Cell {flag:bool,other:bool}
        comptime fn set(p:*bool,value:bool) {p.* = value;}
        comptime fn compute()->bool {
            let cell=Cell {flag:false,other:false};
            let first=cell.ref;let alias=first.flag;
            set(alias,true);set(first.other,alias.*);
            cell.flag and cell.other
        }
        const RESULT=compute();comptime fn run()->bool {RESULT}";
    let package = compile(source);
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "run"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_pointer_capture_mutates_only_live_local_storage() {
    let source = "const RESULT=do {
        let cell=false;let p=cell.ref;
        let action=|| do {p.* = not p.*;p.*};
        let first=action();let second=action();first and not second and not cell
    };comptime fn run()->bool {RESULT}";
    let package = compile(source);
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "run"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_pointer_lifetime_and_constant_escape_are_checked() {
    for (source, anchor) in [
        (
            "const BAD=do {let value=true;value.ref};",
            "storage pointer cannot be materialized",
        ),
        (
            "const BAD=do {let value=true;let p=value.ref;|| p.*};",
            "storage pointer cannot be materialized",
        ),
        (
            "comptime fn dangling()->*bool {let value=true;value.ref}const BAD=dangling().*;",
            "ExpiredPointer",
        ),
        (
            "struct Cell {flag:bool} comptime fn copy()->Cell {let cell=Cell {flag:true};cell.ref.*}const BAD=copy();",
            "Unsupported",
        ),
    ] {
        let failure = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{failure}");
    }
}

#[test]
fn comptime_pointer_failure_does_not_poison_following_invocations() {
    let package = compile(
        "comptime fn dangling()->*bool {let cell=true;cell.ref}
        comptime fn bad()->bool {dangling().*}
        comptime fn good()->bool {let cell=false;let p=cell.ref;p.* = true;cell}",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "bad"), vec![]).unwrap_err().kind,
        comptime_interpretor::ErrorKind::ExpiredPointer
    );
    assert_eq!(
        vm.invoke(id(&package, "good"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_exact_numeric_pointer_updates_preserve_values() {
    let package = compile(
        "typealias Integer=.Integer'builtin; typealias Float=.Float'builtin;
        comptime fn AddInteger(p:*Integer) {p.* = p.* + 12;}
        comptime fn AddFloat(p:*Float) {p.* = p.* + 1.25;}
        comptime fn Run()->bool {
            var integer:Integer=1000000000000000000000000000000000000000000;
            var decimal:Float=1.5;
            AddInteger(integer.ref);
            AddFloat(decimal.ref);
            integer == 1000000000000000000000000000000000000000012 and decimal == 2.75
        }",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for ty in ["Integer", "Float"] {
        let invalid =
            format!("typealias Numeric=.{ty}'builtin; fn bad(p:*Numeric)->Numeric {{p.*}}");
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{ty}"
        );
    }
}

#[test]
fn comptime_mixed_view_replacement_preserves_aliases_and_phase() {
    let source = "typealias Type=.Type'builtin;trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Action=comptime fn(*bool)->bool;impl Duplicate for Action {}
        struct View {action:Action,target:*bool}impl Duplicate for View {}
        comptime fn read(p:*bool)->bool {p.*}
        comptime fn invert(p:*bool)->bool {not p.*}
        comptime fn compute()->bool {
            let first=false;let last=true;var view=View {action:read,target:first.ref};
            let saved=view;let slot=view.ref;slot.* = View {action:invert,target:last.ref};
            view.target.* and view.action(first.ref) and not view.action(view.target)
                and not saved.action(saved.target)
        }
        const VALUE=compute();comptime fn result()->bool {VALUE}
        comptime fn Result()->Type {if compute() {bool} else {.u8'builtin}}
        typealias Chosen=Result();fn run()->Chosen {true}";
    let compile = |source: &str| {
        execution_support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
    };
    let package = compile(source).unwrap();
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "result"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("impl Duplicate for View {}", ""),
        source.replace("impl Duplicate for Action {}", ""),
        source.replace("comptime fn invert(p:*bool)", "fn invert(p:*bool)"),
        source.replace(
            "slot.* = View {action:invert,target:last.ref};",
            "slot.* = do {let temporary=true;View {action:invert,target:temporary.ref}};",
        ),
    ] {
        assert!(
            compile(&invalid).is_err(),
            "accepted invalid replacement: {invalid}"
        );
    }
}

#[test]
fn type_storage_transfer_requires_explicit_copy_and_preserves_type_identity() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for Type {}
        pure comptime fn Identity<T>->Type {T}
        comptime fn Chosen()->Type {
            let selected=Identity<bool>;
            let saved=selected.ref.*;
            selected.ref.* = bool;
            saved
        }
        typealias Result=Chosen();
        comptime fn ResultType()->Type {Result}";
    let compile = |source: &str| {
        execution_support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
    };
    let package = compile(source).unwrap();
    let Value::Type(selected) = luna::invocation::invoke(&package, "ResultType", 100_000).unwrap()
    else {
        panic!("expected Type")
    };
    assert!(
        matches!(selected, nir::Term::NormalForm { .. }),
        "{selected}"
    );
    assert_ne!(selected, nir::Term::Primitive(nir::Primitive::Bool));
    assert_eq!(
        luna::invocation::invoke_with_protocols(&package, "Chosen", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Type(selected)
    );
    assert!(luna::invocation::invoke(&package, "Chosen", 100_000).is_err());

    let missing = compile(&source.replace("impl Duplicate for Type {}", "")).unwrap_err();
    assert!(missing.contains("Copy"), "{missing}");
    let cleanup = compile(&format!(
        "{source}impl Retire for Type {{fn drop(*self) {{}}}}"
    ))
    .unwrap_err();
    assert!(
        cleanup.contains("Copy") || cleanup.contains("comptime-only"),
        "{cleanup}"
    );
}

#[test]
fn explicit_invocation_checks_aggregate_copy_evidence_and_recovers() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Packet {flag:bool}impl Duplicate for Packet {}
        comptime fn compute()->bool {
            let packet=Packet {flag:true};let saved=packet.ref.*;
            packet.ref.* = Packet {flag:false};saved.flag and not packet.flag
        }";
    let package = compile(source);
    assert_eq!(
        luna::invocation::invoke_with_protocols(
            &package,
            "compute",
            "Duplicate",
            "Retire",
            100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    assert!(
        luna::invocation::invoke_with_protocols(&package, "compute", "Duplicate", "Retire", 1)
            .is_err()
    );
    let missing = compile(&source.replace("impl Duplicate for Packet {}", ""));
    let error = luna::invocation::invoke_with_protocols(
        &missing,
        "compute",
        "Duplicate",
        "Retire",
        100_000,
    )
    .unwrap_err();
    assert!(error.contains("pointer Copy evidence"), "{error}");
    let cleanup = compile(&format!(
        "{source}impl Retire for Packet {{fn drop(*self) {{}}}}"
    ));
    assert!(
        luna::invocation::invoke_with_protocols(
            &cleanup,
            "compute",
            "Duplicate",
            "Retire",
            100_000
        )
        .is_err()
    );
    assert_eq!(
        luna::invocation::invoke_with_protocols(
            &package,
            "compute",
            "Duplicate",
            "Retire",
            100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn memoized_type_storage_uses_snapshot_resource_authority() {
    let package=compile("typealias Type=.Type'builtin;
        trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for Type {}
        pure comptime fn Select()->Type {let value=bool;let saved=value.ref.*;value.ref.* = .u8'builtin;saved}");
    let mut items = intrinsic::LangItems::new();
    for (item, name) in [
        (intrinsic::LangItem::Copy, "Duplicate"),
        (intrinsic::LangItem::Drop, "Retire"),
    ] {
        let declaration = package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration;
        items
            .bind_source_traits(&package, &[(item, declaration)])
            .unwrap();
    }
    let prepare = |items: &intrinsic::LangItems| {
        luna::memoization::Invocation::prepare(
            &package,
            items,
            id(&package, "Select"),
            vec![],
            4,
            Default::default(),
        )
        .unwrap()
    };
    let mut allowed = prepare(&items);
    assert!(
        allowed
            .invoke(vec![], &mut comptime_executor::Context::new(1))
            .is_err()
    );
    assert_eq!(allowed.cached_results(), 0);

    for reused in [false, true] {
        let outcome = allowed
            .invoke(vec![], &mut comptime_executor::Context::new(100_000))
            .unwrap();
        assert_eq!(
            *outcome.value,
            Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
        );
        assert_eq!(outcome.reused, reused);
    }
    let mut denied = prepare(&intrinsic::LangItems::new());
    assert!(
        denied
            .invoke(vec![], &mut comptime_executor::Context::new(100_000))
            .is_err()
    );
    assert_eq!(denied.cached_results(), 0);
    assert_eq!(allowed.cached_results(), 1);
}

#[test]
fn memoized_keys_use_resource_evidence_in_source_eq_and_hash() {
    let source = "const eq_key_phase={comptime_phase:true}; trait Eq {^eq_key_phase comptime fn eq(self,other:Self)->bool;}
        const hash_key_phase={comptime_phase:true}; trait Hash {^hash_key_phase comptime fn hash(self)->.u8'builtin;}
        trait Duplicate {}trait Retire {fn drop(*self);}
        struct Packet {flag:bool}impl Duplicate for Packet {}
        comptime impl Eq for bool {comptime fn eq(self,other:bool)->bool {
            let packet=Packet {flag:self};let saved=packet.ref.*;saved.flag == other
        }}
        comptime impl Hash for bool {comptime fn hash(self)->.u8'builtin {
            let packet=Packet {flag:self};let saved=packet.ref.*;
            if saved.flag {0} else {0}
        }}
        typealias Type=.Type'builtin;pure comptime fn Inspect(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    let package = compile(source);
    let prepare = |resources: bool| {
        let mut items = intrinsic::LangItems::new();
        for (item, name) in [
            (intrinsic::LangItem::Eq, "Eq"),
            (intrinsic::LangItem::Hash, "Hash"),
            (intrinsic::LangItem::Copy, "Duplicate"),
            (intrinsic::LangItem::Drop, "Retire"),
        ] {
            if !resources && matches!(item, intrinsic::LangItem::Copy | intrinsic::LangItem::Drop) {
                continue;
            }
            let declaration = package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration;
            items
                .bind_source_traits(&package, &[(item, declaration)])
                .unwrap();
        }
        luna::memoization::Invocation::prepare(
            &package,
            &items,
            id(&package, "Inspect"),
            vec![],
            4,
            Default::default(),
        )
        .unwrap()
    };
    let mut allowed = prepare(true);
    for (value, reused) in [(true, false), (true, true), (false, false), (false, true)] {
        let outcome = allowed
            .invoke(
                vec![Value::Bool(value)],
                &mut comptime_executor::Context::new(100_000),
            )
            .unwrap();
        assert_eq!(
            *outcome.value,
            Value::Type(nir::Term::Primitive(if value {
                nir::Primitive::Bool
            } else {
                nir::Primitive::U8
            }))
        );
        assert_eq!(outcome.reused, reused);
    }
    let mut denied = prepare(false);
    assert!(
        denied
            .invoke(
                vec![Value::Bool(true)],
                &mut comptime_executor::Context::new(100_000)
            )
            .is_err()
    );
    assert_eq!(denied.cached_results(), 0);
}

#[test]
fn memoization_driver_reuses_code_without_caching_non_type_results() {
    let package = compile("pure comptime fn invert(flag:bool)->bool {not flag}");
    let mut invocation = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        id(&package, "invert"),
        vec![],
        4,
        Default::default(),
    )
    .unwrap();
    let mut costs = Vec::new();
    for flag in [true, false, true] {
        let mut context = comptime_executor::Context::new(100_000);
        let result = invocation
            .invoke(vec![Value::Bool(flag)], &mut context)
            .unwrap();
        assert_eq!(*result.value, Value::Bool(!flag));
        assert!(!result.reused);
        assert_eq!(invocation.cached_results(), 0);
        costs.push(100_000 - context.remaining());
    }
    assert!(costs[1] < costs[0], "cold and warm costs: {costs:?}");
    assert_eq!(costs[1], costs[2]);
}

#[test]
fn memoization_driver_retains_generic_preparation_without_result_memoization() {
    let package = compile("pure comptime fn inspect() -> bool where T {true}");
    let mut invocation = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        id(&package, "inspect"),
        vec![nir::Term::Primitive(nir::Primitive::Bool)],
        4,
        Default::default(),
    )
    .unwrap();
    let mut costs = Vec::new();
    for _ in 0..3 {
        let mut context = comptime_executor::Context::new(100_000);
        let result = invocation.invoke(vec![], &mut context).unwrap();
        assert_eq!(*result.value, Value::Bool(true));
        assert!(!result.reused);
        assert_eq!(invocation.cached_results(), 0);
        costs.push(100_000 - context.remaining());
    }
    assert!(costs[1] < costs[0], "generic preparation costs: {costs:?}");
    assert_eq!(costs[1], costs[2]);
}

#[test]
fn generic_code_session_with_ordinary_inputs_reuses_body_not_arguments() {
    use comptime_interpretor::CodeSession;
    use std::sync::Arc;
    let source = Arc::new(compile("pure comptime fn keep(value:T)->T where T {value}"));
    let mut session = CodeSession::new(source.clone());
    for (index, flag) in [true, false, true].into_iter().enumerate() {
        let mut vm = Interpreter::for_comptime(&source, Budget::default()).unwrap();
        let value = vm
            .with_code_session(&mut session, |vm| {
                vm.invoke_instantiated(
                    id(&source, "keep"),
                    vec![nir::Term::Primitive(nir::Primitive::Bool)],
                    vec![Value::Bool(flag)],
                )
            })
            .unwrap();
        assert_eq!(value, Value::Bool(flag));
        assert!(vm.bytecode_invocations() > 0);
        assert_eq!(vm.bytecode_compilations() > 0, index == 0);
    }
}
