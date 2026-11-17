//! Read observations follow storage identity, effects and proven field separation.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use support::{compile, compile_with_protocols};

#[test]
fn implicit_cleanup_invalidates_external_pointer_observations() {
    let prefix = "typealias Byte=.u8'builtin;trait Retire {fn drop(*self);}
        struct Watch {target:*Byte}impl Retire for Watch {fn drop(*self) {self.target.*.* = 0;}}
        fn change(p:*Byte) {let watch=Watch {target:p};}
        fn relay(p:*Byte) {change(p);}";
    for call in ["change(p)", "relay(p)"] {
        let source = format!(
            "{prefix} fn rejected(p:*Byte)->result:bool where ensures result {{let before=p.*;{call};before==p.*}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("postcondition"), "{call}: {error}");
        let checked = format!(
            "{prefix} fn checked(p:*Byte)->result:bool where ensures result {{let before=p.*;{call};assumes before==p.*;before==p.*}}"
        );
        compile_with_protocols(&checked, &Default::default()).unwrap();
    }
    let stable = format!(
        "{prefix} fn read(p:*Byte)->Byte {{p.*}}fn stable(p:*Byte)->result:bool where ensures result {{let before=p.*;read(p);before==p.*}}"
    );
    compile_with_protocols(&stable, &Default::default()).unwrap();
}

#[test]
fn external_pointer_observations_prove_stable_reads_and_bounds() {
    let source="typealias Byte=.u8'builtin;
        fn read(p:*Byte)->result:Byte where ensures result == p.* {p.*}
        fn next(p:*Byte)->result:Byte where requires p.* < 255, ensures result == p.* + 1 {p.* + 1}
        fn repeated(p:*Byte)->result:bool where ensures result {let first=p.*;let second=p.*;first == second}
        fn choose(p:*Byte,flag:bool)->result:Byte where ensures result == p.* {if flag {p.*} else {p.*}}
        fn run()->bool {let value:Byte=41;read(value.ref)==41 and next(value.ref)==42 and repeated(value.ref) and choose(value.ref,true)==41}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("{p.* + 1}", "{p.*}"),
        source.replace("requires p.* < 255, ", ""),
        source.replace("let second=p.*", "let second:.u8'builtin=0"),
    ] {
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn external_pointer_observations_forget_alias_writes_rebindings_and_calls() {
    let prefix =
        "typealias Byte=.u8'builtin;fn clear(p:*Byte) {p.* = 0;}fn pure_read(p:*Byte)->Byte {p.*}";
    for body in [
        "let before=p.*;q.* = 0;before == p.*",
        "let before=p.*;q.*'bitvec[0]=0;before == p.*",
        "let before=p.*;clear(q);before == p.*",
        "var pointer=p;let before=pointer.*;pointer=q;before == pointer.*",
        "let before=p.*;if flag {q.* = 0;}before == p.*",
        "let before=p.*;while flag {q.* = 0;break;}before == p.*",
    ] {
        let source = format!(
            "{prefix}fn rejected(p:*Byte,q:*Byte,flag:bool)->result:bool where ensures result {{{body}}}"
        );
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(failure.contains("postcondition"), "{body}: {failure}");
    }
    let source = format!(
        "{prefix}fn rejected(p:*Byte,q:*Byte)->result:bool where ensures result {{p.* == q.*}}"
    );
    assert!(
        compile_with_protocols(&source, &Default::default()).is_err(),
        "different pointers have no implicit equality"
    );
    let source = format!(
        "{prefix}fn rejected(p:*Byte,q:*Byte)->result:bool where ensures result {{not (p.* == q.*)}}"
    );
    assert!(
        compile_with_protocols(&source, &Default::default()).is_err(),
        "different pointers have no implicit disjointness"
    );
}

#[test]
fn external_pointer_observations_survive_trait_contract_parameter_remapping() {
    let source = "typealias Byte=.u8'builtin;
        trait Read {fn read(self,p:*Byte)->result:Byte where ensures result == p.*;}
        struct Reader {}
        impl Read for Reader {fn read(self,input:*Byte)->Byte {input.*}}
        fn run()->Byte {let value:Byte=41;Reader {}.as(Read).read(value.ref)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(41)
    );
    assert!(
        compile_with_protocols(&source.replace("{input.*}", "{0}"), &Default::default()).is_err()
    );
}

#[test]
fn external_pointer_observations_check_call_inputs_after_argument_effects() {
    let prefix = "typealias Byte=.u8'builtin;
        fn checked(p:*Byte,flag:bool)->Byte where requires p.* < 255 {p.* + 1}
        fn forward(p:*Byte)->Byte where requires p.* < 255 {checked(p,true)}";
    let source = format!(
        "{prefix}
        fn run()->Byte {{let good:Byte=41;let bad:Byte=255;var pointer=good.ref;
            checked(pointer,do {{pointer=bad.ref;true}})}}
        fn forwarded()->Byte {{let good:Byte=41;forward(good.ref)}}"
    );
    let package = compile(&source);
    for entry in ["run", "forwarded"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(42)
        );
    }
    for body in [
        "fn bad()->Byte {var value:Byte=41;checked(value.ref,do {value=255;true})}",
        "fn bad(p:*Byte)->Byte where requires p.* < 255 {checked(p,do {p.* = 255;true})}",
        "fn bad(p:*Byte,q:*Byte)->Byte where requires p.* < 255 {checked(p,do {q.* = 255;true})}",
        "fn bad()->Byte {let value:Byte=255;checked(value.ref,true)}",
    ] {
        let failure =
            compile_with_protocols(&format!("{prefix}{body}"), &Default::default()).unwrap_err();
        assert!(failure.contains("precondition"), "{body}: {failure}");
    }
}

#[test]
fn external_pointer_observations_cross_readonly_defaults() {
    let prefix = "typealias Byte=.u8'builtin;
        fn checked(p:*Byte,.flag:bool=true)->Byte where requires p.* < 255 {p.* + 1}
        fn alias(p:*Byte,.q:*Byte=p)->Byte where requires q.* < 255 {q.* + 1}
        fn forward(p:*Byte)->Byte where requires p.* < 255 {checked(p)}
        fn forward_alias(p:*Byte)->Byte where requires p.* < 255 {alias(p)}";
    let source = format!(
        "{prefix}
        fn run()->Byte {{let value:Byte=41;checked(value.ref)}}
        fn direct_alias()->Byte {{let value:Byte=41;alias(value.ref)}}
        fn forwarded()->Byte {{let value:Byte=41;forward(value.ref)}}
        fn forwarded_alias()->Byte {{let value:Byte=41;forward_alias(value.ref)}}"
    );
    let package = compile(&source);
    for entry in ["run", "direct_alias", "forwarded", "forwarded_alias"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(42)
        );
    }
    assert!(
        compile_with_protocols(
            &source.replace("value:Byte=41", "value:Byte=255"),
            &Default::default()
        )
        .unwrap_err()
        .contains("precondition")
    );
    for default in ["do {p.* = 255;true}", "write(p)"] {
        let invalid = format!(
            "typealias Byte=.u8'builtin;fn write(p:*Byte)->bool {{p.* = 255;true}}
            fn checked(p:*Byte,.flag:bool={default})->Byte where requires p.* < 255 {{p.* + 1}}
            fn run()->Byte {{let value:Byte=41;checked(value.ref)}}"
        );
        assert!(
            compile_with_protocols(&invalid, &Default::default())
                .unwrap_err()
                .contains("P3 stability")
        );
    }
}

#[test]
fn external_pointer_observations_reject_effectful_default_ir() {
    use elaboration::verification::{Limits, flow};
    use nir::program::{Expr, ExprKind as K, Literal, Place};
    let source="typealias Byte=.u8'builtin;
        fn checked(p:*Byte,other:*Byte,.flag:bool=true,.q:*Byte=p)->Byte where requires q.* < 255 {q.* + 1}
        fn run()->Byte {let value:Byte=41;let bad:Byte=255;checked(value.ref,bad.ref)}";
    for rebind in [false, true] {
        let mut package = compile(source);
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.name == "run")
            .unwrap();
        let K::Block {
            result: Some(call), ..
        } = &mut function.body.kind
        else {
            panic!("block")
        };
        let K::Call {
            arguments,
            parameter_bindings,
            ..
        } = &mut call.kind
        else {
            panic!("call")
        };
        let pointer_type = arguments[0].value.ty.clone();
        let default = arguments.iter_mut().find(|a| a.parameter == 2).unwrap();
        let span = default.value.source;
        let write = if rebind {
            K::Assign {
                place: Place {
                    root: parameter_bindings[0],
                    fields: vec![],
                },
                value: Box::new(Expr {
                    source: span,
                    ty: pointer_type,
                    kind: K::Local(parameter_bindings[1]),
                }),
            }
        } else {
            K::WritePointer {
                pointer: Box::new(Expr {
                    source: span,
                    ty: pointer_type,
                    kind: K::Local(parameter_bindings[0]),
                }),
                value: Box::new(Expr {
                    source: span,
                    ty: nir::Term::Primitive(nir::Primitive::U8),
                    kind: K::Literal(Literal::UnsignedInteger(255)),
                }),
            }
        };
        default.value.kind = K::Block {
            statements: vec![Expr {
                source: span,
                ty: nir::Term::Primitive(nir::Primitive::Void),
                kind: write,
            }],
            result: Some(Box::new(Expr {
                source: span,
                ty: default.value.ty.clone(),
                kind: K::Literal(Literal::Bool(true)),
            })),
        };
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let failure = flow::analyze_in_package(&package, function, &mut Limits::default())
            .err()
            .expect("effectful default cannot reuse input observations");
        assert!(failure.to_string().contains("precondition"), "{failure}");
    }
}

#[test]
fn external_pointer_observations_survive_proved_readonly_calls() {
    let source = "typealias Byte=.u8'builtin;
        fn read(p:*Byte)->Byte {p.*}
        fn identity(value:T)->T where T {value}
        fn defaulted(.flag:bool=true)->bool {flag}
        fn next(p:*Byte)->Byte where requires p.* < 255 {p.* + 1}
        fn stable(p:*Byte)->result:bool where ensures result {
            let before=p.*;read(p);identity(true);defaulted();before == p.*
        }
        fn forward(p:*Byte)->Byte where requires p.* < 255 {read(p);next(p)}
        fn run()->bool {let value:Byte=41;stable(value.ref) and forward(value.ref)==42}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for call in ["write(q)", "unknown(q)", "write(do {q})"] {
        let invalid = format!(
            "typealias Byte=.u8'builtin;
            fn write(p:*Byte) {{p.* = 0;}}
            fn rejected(p:*Byte,q:*Byte,unknown:fn(*Byte))->result:bool where ensures result {{
                let before=p.*;{call};before == p.*
            }}"
        );
        let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(failure.contains("postcondition"), "{failure}");
    }
    let invalid = "typealias Byte=.u8'builtin;
        fn read(p:*Byte)->Byte {p.*}
        fn rejected(p:*Byte,q:*Byte)->result:bool where ensures result {
            let before=p.*;read(do {q.* = 0;q});before == p.*
        }";
    assert!(
        compile_with_protocols(invalid, &Default::default())
            .unwrap_err()
            .contains("postcondition")
    );
}

#[test]
fn external_pointer_observations_track_nested_field_paths() {
    let source="typealias Byte=.u8'builtin;
        struct Inner {value:Byte,other:Byte}
        struct Outer {left:Inner,right:Inner}
        fn next(p:*Byte)->Byte where requires p.* < 255 {p.* + 1}
        fn read(p:*Outer)->result:Byte where ensures result == p.left.value.* {p.left.value.*}
        fn stable(p:*Outer)->result:bool where ensures result {
            let before=p.left.value.*;let other=p.right.value.*;before == p.left.value.*
        }
        fn forward(p:*Outer)->Byte where requires p.left.value.* < 255 {next(p.left.value)}
        fn forward_outer(p:*Outer)->Byte where requires p.left.value.* < 255 {forward(p)}
        fn default_outer(p:*Outer,.q:*Outer=p)->Byte where requires q.left.value.* < 255 {forward(q)}
        fn run()->bool {let value=Outer {left:Inner {value:41,other:7},right:Inner {value:255,other:0}};
            stable(value.ref) and read(value.ref)==41 and forward_outer(value.ref)==42 and default_outer(value.ref)==42}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    // Distinct structural fields have disjoint storage. Writing either sibling
    // keeps the observed left.value unchanged; also check the write took place.
    for (write, observed_write) in [
        ("p.right.value.* = 0;", "value.right.value==0"),
        ("p.left.other.* = 0;", "value.left.other==0"),
    ] {
        let modified = source.replace("let other=p.right.value.*;", write).replace(
            "default_outer(value.ref)==42",
            &format!("default_outer(value.ref)==42 and {observed_write}"),
        );
        let package = compile(&modified);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{write}"
        );
    }
    // Reassigning the same origin preserves the address despite changing the
    // local binding version. A genuinely different origin must not inherit it.
    let rebound = source
        .replace(
            "let before=p.left.value.*;",
            "var pointer=p;let before=pointer.left.value.*;",
        )
        .replace(
            "let other=p.right.value.*;before == p.left.value.*",
            "pointer=p;before == pointer.left.value.*",
        );
    let package = compile(&rebound);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let different = rebound
        .replace("fn stable(p:*Outer)", "fn stable(p:*Outer,q:*Outer)")
        .replace("pointer=p;before", "pointer=q;before")
        .replace("stable(value.ref)", "stable(value.ref,value.ref)");
    for (invalid, diagnostic) in [
        (different, "postcondition"),
        (
            source.replace("{p.left.value.*}", "{p.right.value.*}"),
            "postcondition",
        ),
        (
            source.replace("next(p.left.value)", "next(p.right.value)"),
            "precondition",
        ),
        (
            source.replace("let other=p.right.value.*;", "p.left.value.* = 0;"),
            "postcondition",
        ),
    ] {
        let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(failure.contains(diagnostic), "{invalid}: {failure}");
    }
    let invalid=format!("{source}
        fn checked(p:*Outer,flag:bool)->Byte where requires p.left.value.* < 255 {{p.left.value.* + 1}}
        fn rejected()->Byte {{var value=Outer {{left:Inner {{value:41,other:7}},right:Inner {{value:0,other:0}}}};
            checked(value.ref,do {{value.left.value=255;true}})}}");
    assert!(
        compile_with_protocols(&invalid, &Default::default())
            .unwrap_err()
            .contains("precondition")
    );
}

#[test]
fn external_pointer_observations_track_anonymous_field_paths() {
    let source="comptime fn schema(...fields:T)->T where T {fields}
        fn make(...fields:T)->T where T {fields}
        const SAMPLE=schema {flag:true,nested:schema {other:false}};typealias Options=SAMPLE'type;
        fn checked(p:*bool)->bool where requires p.* {p.*}
        fn read(p:*Options)->result:bool where ensures result == p.nested.other.* {p.nested.other.*}
        fn forward(p:*Options)->bool where requires p.flag.* {checked(p.flag)}
        fn outer(p:*Options)->bool where requires p.flag.* {forward(p)}
        fn run()->bool {let value=make {flag:true,nested:make {other:false}};assumes value.flag;outer(value.ref) and not read(value.ref)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("checked(p.flag)", "checked(p.nested.other)"),
        source.replace("{p.nested.other.*}", "{p.flag.*}"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn addressed_local_writes_invalidate_opaque_pointer_observations() {
    for write in ["value=42", "value'bitvec[0]=0"] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn bad(alias:fn(*Byte)->*Byte)->result:bool where ensures result {{
                var value:Byte=41;let pointer=alias(value.ref);let before=pointer.*;
                {write};before == pointer.*
            }}"
        );
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            failure.contains("postcondition") && failure.contains("not proven"),
            "{failure}"
        );
    }
    let field = "typealias Byte=.u8'builtin;struct Cell {value:Byte}
        fn bad(alias:fn(*Byte)->*Byte)->result:bool where ensures result {
            var cell=Cell {value:41};let pointer=alias(cell.value.ref);let before=pointer.*;
            cell.value=42;before == pointer.*
        }";
    let failure = compile_with_protocols(field, &Default::default()).unwrap_err();
    assert!(
        failure.contains("postcondition") && failure.contains("not proven"),
        "{failure}"
    );
}
