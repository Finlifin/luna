//! Exercise the public meta Object interface with the repository library source.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Interpreter, Value};
use nir::{Primitive, Term};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        source,
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

fn invoke(source: &str) -> Result<Value, String> {
    let meta = include!("support/meta_source.rs");
    let package = compile(&format!("mod meta {{ {meta} }} {source}"))?;
    let mut vm =
        Interpreter::for_comptime(&package, Default::default()).map_err(|e| e.to_string())?;
    vm.invoke(support::id(&package, "run"), vec![])
        .map_err(|e| e.to_string())
}

#[test]
fn named_object_reads_use_the_last_domain_and_preserve_optional_layers() {
    let value = invoke("
        pure comptime fn Identity<T>->.Type'builtin {T}
        comptime fn run()->bool {
            let absent:?bool=null;
            let object={flag:true, 'x', flag:false, empty:absent, nested:{flag:true}, kind:Identity<bool>};
            let shadowed={flag:true, flag:'x'};
            (meta.Object.get<bool>(object,.flag) matches false?)
                and (meta.Object.get<bool>(object,.missing) matches null)
                and (meta.Object.get<bool>(shadowed,.flag) matches null)
                and (meta.Object.get<.char'builtin>(shadowed,.flag) matches 'x'?)
                and (meta.Object.get<?bool>(object,.empty) match {value?=>value matches null,null=>false})
                and (meta.Object.get<meta.Object>(object,.nested) match {
                    nested?=>meta.Object.get<bool>(nested,.flag) matches true?,null=>false})
                and (meta.Object.get<.Type'builtin>(object,.kind) match {
                    T?=>T==Identity<bool>,null=>false})
                and (meta.Object.get<bool>({},.missing) matches null)
        }
    ").unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn named_object_reads_keep_argument_order_early_return_and_snapshot() {
    let value = invoke(
        "
        typealias Integer = .Integer'builtin;
        typealias Symbol = .Symbol'builtin;
        struct Trace { visits: Integer }
        impl Trace {
            comptime fn make(self) -> meta.Object {
                self.visits = self.visits * 10 + 1;
                let object = {flag: true};
                object
            }
            comptime fn key(self) -> Symbol {
                self.visits = self.visits * 10 + 2;
                .flag
            }
        }
        comptime fn run() -> bool {
            let state = Trace {visits: 0};
            let value = meta.Object.get<bool>(state.make(), state.key());
            var original = {flag: true};
            let snapshot = original;
            let current = meta.Object.get<bool>(original,
                do {original = {flag: false}; .flag});
            state.visits == 12 and (value matches true?) and (current matches false?)
                and (snapshot.get<bool>(.flag) matches true?)
                and (original.get<bool>(.flag) matches false?)
        }
    ",
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
    assert_eq!(
        invoke(
            "comptime fn run()->bool {
        meta.Object.get<bool>(do {return true;},do {while true {} .flag}); false
    }"
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn named_object_queries_reject_wrong_keys_and_runtime_use() {
    let meta = include!("support/meta_source.rs");
    for source in [
        "comptime fn bad()->?bool {meta.Object.get<bool>({flag:true},0)}",
        "fn bad()->?bool {meta.Object.get<bool>({flag:true},.flag)}",
        "comptime fn bad()->?bool {.object_get'builtin(bool,{flag:true})}",
    ] {
        assert!(
            compile(&format!("mod meta {{ {meta} }} {source}")).is_err(),
            "{source}"
        );
    }
    let package = compile(&format!(
        "mod meta {{ {meta} }} comptime fn run()->bool {{
        meta.Object.get<bool>({{flag:true}},.flag) matches true?
    }}"
    ))
    .unwrap();
    assert!(package.to_string().contains("named-value"));
    assert!(luna::invocation::invoke(&package, "run", 1).is_err());
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn named_lookup_avoids_repeated_object_transport_during_feature_search() {
    let meta = include!("support/meta_source.rs");
    let fields = (0..64)
        .map(|index| format!("f{index}:true"))
        .collect::<Vec<_>>()
        .join(",");
    let package = compile(&format!(
        "mod meta {{ {meta} }}
        comptime fn previous(object:meta.Object,key:.Symbol'builtin)->?bool {{
            let count=meta.Object.length(object);
            var index:.Integer'builtin=0;
            while index < count {{
                if meta.Object.key(object,index) match {{name?=>name==key,null=>false}} {{
                    return meta.Object.value<bool>(object,index);
                }}
                index += 1;
            }}
            null
        }}
        comptime fn previous_entry()->?bool {{previous({{{fields}}},.f63)}}
        comptime fn named_entry()->?bool {{meta.Object.get<bool>({{{fields}}},.f63)}}
    "
    ))
    .unwrap();
    let mut costs = Vec::new();
    let mut values = Vec::new();
    for entry in ["previous_entry", "named_entry"] {
        let mut vm = Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        let before = vm.remaining_steps();
        values.push(vm.invoke(support::id(&package, entry), vec![]).unwrap());
        costs.push(before - vm.remaining_steps());
    }
    assert_eq!(values[0], values[1]);
    assert!(
        matches!(&values[1],Value::Optional{value:Some(value),..} if **value==Value::Bool(true))
    );
    eprintln!(
        "64 unique features, last key: previous={} named={}",
        costs[0], costs[1]
    );
    assert!(costs[1] * 2 < costs[0], "{costs:?}");
}

#[test]
fn indexed_object_queries_preserve_order_domains_and_optional_layers() {
    let value = invoke(
        "
        comptime fn run()->bool {
            let absent:?bool=null;
            let nested={enabled:true};
            let object={flag:true, 'x', flag:false, absent, nested};
            let first=meta.Object.value<bool>(object,0);
            let last=meta.Object.value<bool>(object,2);
            let missing=meta.Object.value<bool>(object,1);
            let layered=meta.Object.value<?bool>(object,3);
            let inner=meta.Object.value<meta.Object>(object,4);
            meta.Object.length(object)==5
                and (meta.Object.key(object,0) match {key?=>key==.flag,null=>false})
                and (meta.Object.key(object,1) match {_?=>false,null=>true})
                and (meta.Object.key(object,2) match {key?=>key==.flag,null=>false})
                and meta.Object.EntryType(object,1)==.char'builtin
                and (first match {value?=>value,null=>false})
                and (last match {value?=>not value,null=>false})
                and (missing match {_?=>false,null=>true})
                and (layered match {value?=>value match {_?=>false,null=>true},null=>false})
                and (inner match {value?=>meta.Object.length(value)==1,null=>false})
        }
    ",
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn object_queries_preserve_type_identity_and_evaluate_inputs_once_in_order() {
    let value=invoke("
        struct A {} struct B {}
        comptime fn make(state:*bool)->meta.Object {state.* = not state.*; let item=A {}; let result={stored:item}; result}
        comptime fn index(state:*bool)->.Integer'builtin {
            if state.* {state.* = false; return 0;} 9
        }
        comptime fn run()->bool {
            var state=false;
            let got=meta.Object.value<A>(make(state.ref),index(state.ref));
            let item=A {};
            let object={stored:item, kind:.u8'builtin};
            not state and (got match {_?=>true,null=>false})
                and (meta.Object.value<B>(object,0) match {_?=>false,null=>true})
                and (meta.Object.value<.Type'builtin>(object,1) match {T?=>T==.u8'builtin,null=>false})
        }
    ").unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn object_queries_reject_bounds_and_runtime_transport() {
    let meta = include!("support/meta_source.rs");
    for index in ["-1", "1", "99999999999999999999999999999999999"] {
        for query in ["Object.key", "Object.EntryType", "Object.value<bool>"] {
            let error = invoke(&format!(
                "comptime fn run()->bool {{let value={{true}}; meta.{query}(value,{index}); true}}"
            ))
            .unwrap_err();
            assert!(error.contains("IndexOutOfBounds"), "{error}");
        }
    }
    let error = compile(&format!(
        "mod meta {{ {meta} }} fn bad()->.Integer'builtin {{meta.Object.length({{true}})}}"
    ))
    .unwrap_err();
    assert!(
        error.contains("comptime") || error.contains("phase"),
        "{error}"
    );
    assert_eq!(
        invoke("comptime fn run()->bool {meta.Object.length({})==0}").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn object_query_argument_return_skips_later_inputs() {
    assert_eq!(
        invoke(
            "comptime fn run()->bool {
        meta.Object.key(do {return true;}, do {while true {} 0}); false
    }"
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn code_generators_can_reconstruct_nested_object_data_without_normal_term_erasure() {
    let meta = include!("support/meta_source.rs");
    let package = compile(&format!(
        "mod meta {{ {meta} }}
        comptime fn generate()-><|-bool> {{
            let nested={{flag:true}};
            let object={{nested:nested, tag:.sample}};
            let selected=meta.Object.value<meta.Object>(object,0);
            let answer=selected match {{
                inner?=>meta.Object.value<bool>(inner,0) match {{flag?=>flag,null=>false}},
                null=>false,
            }};
            '(answer)
        }}
        fn run()->bool {{~<generate()>}}
    "
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );

    let failure = compile(&format!(
        "mod meta {{ {meta} }}
        comptime fn generate()-><|-bool> {{
            var flag=true; let object={{pointer:flag.ref}}; '(true)
        }} fn run()->bool {{~<generate()>}}
    "
    ))
    .unwrap_err();
    assert!(
        failure.contains("frozen") || failure.contains("pointer"),
        "{failure}"
    );
}

#[test]
fn repository_package_exports_object_for_user_attribute_constructors() {
    let directory = std::env::temp_dir().join(format!("luna-meta-object-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.fl");
    std::fs::write(
        &input,
        "use meta.prelude.*;
        pure comptime fn marker() -> Object { let value = { enabled: true }; value }
        ^marker() fn example() {}",
    )
    .unwrap();
    let library =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/meta/lib.fl");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .args(["--packages-v1", "--package", "1", "101", "app"])
        .arg(&input)
        .args(["--builtin-impl-package", "2"])
        .args(["--package", "2", "202", "meta"])
        .arg(&library)
        .args(["--package", "3", "303", "builtin"])
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../library/builtin/primitives.fl"),
        )
        .args(["--dependency", "2", "builtin", "3"])
        .args(["--package", "4", "404", "attrs"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/attrs/lib.fl"))
        .args(["--dependency", "2", "attrs", "4"])
        .args(["--dependency", "4", "builtin", "3"])
        .args(nir::intrinsics::Operation::ALL.iter().flat_map(|op| {
            [
                "--comptime-intrinsic".to_owned(),
                format!(
                    "{}=meta.numeric.scalar_math.target.Scalar.{}",
                    op.name(),
                    op.name()
                ),
            ]
        }))
        .args([
            "--dependency",
            "1",
            "meta",
            "2",
            "--primary",
            "1",
            "--meta-list-type",
            "meta.List",
            "--emit",
            "attributes",
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = String::from_utf8_lossy(&output.stdout);
    let user_attributes = dump.matches("(property \"enabled\")").count();
    let phase_attributes = dump.matches("(property \"comptime_phase\")").count();
    assert_eq!(user_attributes, 1, "{dump}");
    // Comptime-only traits no longer need per-member phase attributes.
    // Any explicit library attributes must still be accounted for below.
    assert_eq!(
        dump.matches("(evaluated-attribute ").count(),
        user_attributes + phase_attributes,
        "{dump}"
    );
    assert!(dump.contains("(property \"enabled\")"), "{dump}");
    assert!(dump.contains("(bool true)"), "{dump}");
}

#[test]
fn public_object_preserves_order_duplicate_keys_and_nested_constants() {
    let meta = include!("support/meta_source.rs");
    let package = compile(&format!(
        "mod meta {{ {meta} }}
         use meta.prelude.*;
         const saved = {{ flag: true, 'x', flag: false }};
         comptime fn identity(value: meta.Object) -> Object {{ value }}
         comptime fn metadata() -> Object {{
             let result = {{ nested: saved, mode: .c }};
             identity(result)
         }}"
    ))
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm
        .invoke(support::id(&package, "metadata"), vec![])
        .unwrap();
    let Value::Object(object) = value else {
        panic!("expected Object")
    };
    assert_eq!(object.entries().len(), 2);
    let nested = &object.entries()[0];
    assert_eq!(nested.key(), Some("nested"));
    assert_eq!(nested.value().ty, Term::Primitive(Primitive::Object));
    let Value::Object(nested) = &nested.value().value else {
        panic!("expected nested Object")
    };
    assert_eq!(
        nested
            .entries()
            .iter()
            .map(|entry| entry.key())
            .collect::<Vec<_>>(),
        vec![Some("flag"), None, Some("flag")]
    );
    assert_eq!(nested.entries()[0].value().value, Value::Bool(true));
    assert_eq!(nested.entries()[1].value().value, Value::Char('x'));
    assert_eq!(nested.entries()[2].value().value, Value::Bool(false));
    assert_eq!(object.entries()[1].key(), Some("mode"));
    assert_eq!(
        object.entries()[1].value().ty,
        Term::Primitive(Primitive::Symbol)
    );
    assert_eq!(object.entries()[1].value().value, Value::Symbol("c".into()));
}

#[test]
fn public_alias_does_not_allow_runtime_transport_or_wrong_values() {
    let meta = include!("support/meta_source.rs");
    for body in [
        "fn invalid(value: Object) {}",
        "fn invalid() -> Object { let value = { flag: true }; value }",
        "comptime fn invalid() -> Object { true }",
    ] {
        let source = format!("mod meta {{ {meta} }} use meta.prelude.*; {body}");
        assert!(compile(&source).is_err(), "{body}");
    }
}

#[test]
fn object_edits_preserve_snapshots_order_keys_and_replacement_domains() {
    let value = invoke("
        comptime fn run()->bool {
            let original={flag:true, 'x', flag:false};
            let key:.Symbol'builtin=.flag;
            let appended=meta.Object.append(original,.u8'builtin,.key = key);
            let positional=meta.Object.append(appended,false);
            let replaced=meta.Object.replace(positional,0,'z');
            let removed=meta.Object.remove(replaced,1);
            let joined=meta.Object.concat(removed,original);
            meta.Object.length(original)==3
                and meta.Object.length(joined)==7
                and meta.Object.EntryType(original,0)==bool
                and meta.Object.EntryType(replaced,0)==.char'builtin
                and meta.Object.EntryType(joined,2)==.Type'builtin
                and (meta.Object.key(joined,0) match {v?=>v==.flag,null=>false})
                and (meta.Object.key(joined,2) match {v?=>v==.flag,null=>false})
                and (meta.Object.key(joined,3) match {_?=>false,null=>true})
                and (meta.Object.value<.char'builtin>(joined,0) match {v?=>v=='z',null=>false})
                and (meta.Object.value<.Type'builtin>(joined,2) match {T?=>T==.u8'builtin,null=>false})
                and (meta.Object.value<bool>(joined,4) match {v?=>v,null=>false})
                and (meta.Object.value<bool>(replaced,0) match {_?=>false,null=>true})
        }
    ").unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn object_edits_check_indices_and_runtime_phase() {
    let meta = include!("support/meta_source.rs");
    for index in ["-1", "1", "9999999999999999999999999999999999"] {
        for operation in [
            format!("meta.Object.remove(object,{index})"),
            format!("meta.Object.replace(object,{index},false)"),
        ] {
            let error = invoke(&format!(
                "comptime fn run()->meta.Object {{let object={{flag:true}}; {operation}}}"
            ))
            .unwrap_err();
            assert!(error.contains("IndexOutOfBounds"), "{error}");
        }
    }
    let error=compile(&format!("mod meta {{{meta}}} fn run()->meta.Object {{let object={{flag:true}};meta.Object.remove(object,0)}}")).unwrap_err();
    assert!(error.contains("comptime"), "{error}");
}

#[test]
fn object_edits_evaluate_once_in_order_and_keep_optional_payloads() {
    let value=invoke("
        comptime fn make(state:*bool)->meta.Object {state.* = not state.*; let object={flag:true}; object}
        comptime fn index(state:*bool)->.Integer'builtin {if state.* {state.* = false; return 0;} 9}
        comptime fn run()->bool {
            var state=false;
            let absent:?bool=null;
            let replaced=meta.Object.replace(make(state.ref),index(state.ref),absent);
            let empty=meta.Object.remove(replaced,0);
            let combined=meta.Object.concat(empty,replaced);
            not state and meta.Object.length(empty)==0
                and (meta.Object.value<?bool>(combined,0) match {v?=>v match {_?=>false,null=>true},null=>false})
        }
    ").unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn object_edit_return_skips_later_inputs() {
    assert_eq!(invoke("comptime fn run()->bool {meta.Object.replace(do {return true;},do {while true {} 0},false);false}").unwrap(),Value::Bool(true));
}

#[test]
fn literal_keys_receive_optional_symbol_context_without_enum_resolution() {
    let value = invoke(
        "
        comptime fn nested(key:??.Symbol'builtin)->bool {
            key match {inner?=>inner match {v?=>v==.flag,null=>false},null=>false}
        }
        comptime fn run()->bool {
            let base={true};
            let object=meta.Object.append(base,false,.key = .flag);
            nested(.flag) and (meta.Object.key(object,1) match {v?=>v==.flag,null=>false})
        }
    ",
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn objects_are_frozen_normal_form_inputs_with_ordered_typed_identity() {
    let value = invoke(
        "
        pure comptime fn Tagged<metadata:meta.Object>->.Type'builtin {bool}
        comptime fn read()->bool where metadata:meta.Object {
            meta.Object.length(metadata)==3
                and (meta.Object.value<bool>(metadata,0) match {v?=>v,null=>false})
                and meta.Object.EntryType(metadata,1)==.Type'builtin
                and (meta.Object.value<.Type'builtin>(metadata,1) match {T?=>T==bool,null=>false})
                and (meta.Object.key(metadata,2) match {v?=>v==.flag,null=>false})
        }
        const checked=read<{flag:true, kind:bool, flag:false}>();
        comptime fn run()->bool {
            let left={flag:true, kind:bool, flag:false};
            let same={flag:true, kind:bool, flag:false};
            Tagged<left> == Tagged<same>
                and checked
        }
    ",
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn frozen_object_identity_keeps_domains_and_nested_optional_layers() {
    let value = invoke(
        "
        pure comptime fn Tagged<metadata:meta.Object>->.Type'builtin {bool}
        pure comptime fn Identity<T>->.Type'builtin {T}
        comptime fn run()->meta.Object {
            let small:.u8'builtin=1;
            let wide:.Integer'builtin=1;
            let a={value:small};
            let b={value:wide};
            let x:?bool=null;
            let y:??bool=x;
            let inner={flag:true};
            let first={inner:inner, value:x, kind:Identity<bool>};
            let same={inner:{flag:true}, value:x, kind:Identity<bool>};
            let other={inner:inner, value:y, kind:bool};
            let reordered={value:x, inner:inner, kind:Identity<bool>};
            let result={Tagged<a>, Tagged<b>, Tagged<first>, Tagged<same>, Tagged<other>, bool, Tagged<reordered>};
            result
        }
    ",
    )
    .unwrap();
    let Value::Object(values) = value else {
        panic!()
    };
    let entries = values.entries();
    assert_ne!(entries[0].value().value, entries[1].value().value);
    assert_eq!(entries[2].value().value, entries[3].value().value);
    assert_ne!(entries[2].value().value, entries[4].value().value);
    assert_ne!(entries[0].value().value, entries[5].value().value);
    assert_ne!(entries[2].value().value, entries[6].value().value);
    let Value::Type(Term::NormalForm { .. }) = &entries[0].value().value else {
        panic!()
    };
}

#[test]
fn object_normal_inputs_do_not_freeze_active_pointers() {
    let error = invoke(
        "
        pure comptime fn Tagged<metadata:meta.Object>->.Type'builtin {bool}
        comptime fn run()->bool {
            var flag=true;
            let metadata={pointer:flag.ref};
            Tagged<metadata> == bool
        }
    ",
    )
    .unwrap_err();
    assert!(
        error.contains("Unsupported")
            || error.contains("freeze")
            || error.contains("comptime input"),
        "{error}"
    );
}

#[test]
fn object_clause_inputs_reconstruct_nested_trusted_lists() {
    let value = invoke(
        "
        const FLAGS:meta.List<bool> = [true,false];
        const METADATA={flags:FLAGS};
        comptime fn read()->bool where metadata:meta.Object {
            meta.Object.value<meta.List<bool>>(metadata,0) match {
                values?=>values.length()==2 and values[0] and not values[1],
                null=>false,
            }
        }
        const CHECKED=read<METADATA>();
        comptime fn run()->bool {CHECKED}
    ",
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn inherent_object_methods_preserve_domains_duplicates_and_snapshots() {
    let value = invoke(
        "comptime fn run()->bool {
            let absent:?bool=null;
            let original={flag:true, 'x', flag:false, empty:absent};
            let added=original.append(true,.key=.flag);
            let changed=added.replace(1,false);
            let removed=changed.remove(0);
            let joined=removed.concat({tail:true});
            original.length()==4 and joined.length()==5
                and (original.get<bool>(.flag) matches false?)
                and (added.get<bool>(.flag) matches true?)
                and (original.key(1) matches null)
                and (original.value<bool>(1) matches null)
                and (changed.value<bool>(1) matches false?)
                and changed.EntryType(1)==bool
                and (original.get<?bool>(.empty) match {value?=>value matches null,null=>false})
                and (joined.get<bool>(.tail) matches true?)
        }",
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn inherent_object_methods_evaluate_receiver_and_arguments_once() {
    let meta = include!("support/meta_source.rs");
    let source = format!(
        "mod meta {{{meta}}}
        comptime fn make()->meta.Object {{let value={{flag:true}};value}}
        comptime fn produce_key()->meta.builtin.Symbol {{.flag}}
        comptime fn run()->bool {{make().get<bool>(produce_key()) matches true?}}"
    );
    let package = compile(&source).unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let ids = [
        support::id(&package, "make"),
        support::id(&package, "produce_key"),
    ];
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            comptime_interpretor::Event::EnterFunction(id) => {
                ids.iter().position(|expected| id == expected)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, [0, 1]);
    for source in [
        "fn bad(value:meta.Object)->bool {value.get<bool>(.flag) matches true?}",
        "comptime fn bad()->?bool {let value={flag:true};value.get<bool>(0)}",
        "comptime fn bad()->bool {let value={flag:true};value.get(.flag)}",
    ] {
        assert!(
            compile(&format!("mod meta {{{meta}}}{source}")).is_err(),
            "{source}"
        );
    }
}

#[test]
fn object_set_updates_value_receivers_and_preserves_saved_snapshots() {
    assert_eq!(
        invoke(
            "struct Draft { entries: meta.Object }
        impl Draft {
            comptime fn reset(self) -> bool {
                self.entries = {fresh: false};
                true
            }
        }
        comptime fn run() -> bool {
            var object = {flag: false, flag: false};
            let saved = object;
            object.set(0, true);
            object.set(1, 'x');
            let draft = Draft { entries: object };
            draft.entries.set(0, draft.reset());
            (saved.value<bool>(0) matches false?)
                and (object.value<bool>(0) matches true?)
                and (object.get<bool>(.flag) matches null)
                and (object.key(1) matches .flag?)
                and draft.entries.length() == 1
                and (draft.entries.get<bool>(.fresh) matches true?)
        }"
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn object_set_rejects_bad_indices_and_immutable_rebinding() {
    for body in [
        "var object = {flag: false}; object.set(-1, true);",
        "var object = {flag: false}; object.set(1, true);",
        "var object = {}; object.set(0, true);",
        "let object = {flag: false}; object.set(0, true);",
    ] {
        assert!(
            invoke(&format!("comptime fn run() -> bool {{ {body} true }}")).is_err(),
            "{body}"
        );
    }
    // A fresh invocation still accepts a valid edit after rejected programs.
    assert_eq!(
        invoke(
            "comptime fn run() -> bool {
        var object = {flag: false}; object.set(0, true);
        object.get<bool>(.flag) matches true?
    }"
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn object_ranges_preserve_order_domains_duplicates_and_snapshots() {
    let source = include_str!("../../../development/examples/sysroot-meta-object-methods.fl");
    assert_eq!(invoke(&format!("typealias Object=meta.Object;typealias Type=.Type'builtin;typealias char=.char'builtin;{source}")).unwrap(),Value::Bool(true));
}

#[test]
fn object_range_failures_leave_inputs_usable_and_vm_recovers() {
    let meta = include!("support/meta_source.rs");
    let package = compile(&format!(
        "mod meta {{{meta}}}
        typealias Integer=.Integer'builtin;
        comptime fn range_slice(start:Integer,end:Integer)->meta.Object {{
            let input={{flag:true,'x',flag:false}};input.slice(start,end)
        }}
        comptime fn range_replace(start:Integer,end:Integer)->meta.Object {{
            let input={{flag:true,'x',flag:false}};input.replace_range(start,end,{{false}})
        }}"
    ))
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let integer = |value| Value::Integer(comptime_runtime::Integer::from_i128(value));
    for name in ["range_slice", "range_replace"] {
        for (start, end) in [
            (-1, 0),
            (0, -1),
            (2, 1),
            (0, 4),
            (4, 4),
            (i128::MAX, i128::MAX),
        ] {
            assert_eq!(
                vm.invoke(
                    support::id(&package, name),
                    vec![integer(start), integer(end)]
                )
                .unwrap_err()
                .kind,
                comptime_interpretor::ErrorKind::IndexOutOfBounds
            );
        }
        let before = vm.bytecode_invocations();
        let Value::Object(value) = vm
            .invoke(support::id(&package, name), vec![integer(0), integer(3)])
            .unwrap()
        else {
            panic!("Object")
        };
        assert_eq!(
            value.entries().len(),
            if name == "range_slice" { 3 } else { 1 }
        );
        assert_eq!(
            vm.bytecode_invocations() - before,
            if name == "range_slice" { 2 } else { 10 }
        );
    }
    for body in [
        "fn bad(value:meta.Object)->meta.Object {value.slice(0,0)}",
        "comptime fn bad(value:meta.Object)->meta.Object {value.slice(false,0)}",
        "comptime fn bad(value:meta.Object)->meta.Object {value.slice(0,true)}",
        "comptime fn bad(value:meta.Object)->meta.Object {value.replace_range(0,0,false)}",
    ] {
        assert!(
            compile(&format!("mod meta {{{meta}}}{body}")).is_err(),
            "{body}"
        );
    }
}

#[test]
fn object_range_arguments_run_once_in_order_and_return_skips_edit() {
    assert_eq!(invoke("typealias Integer=.Integer'builtin;typealias char=.char'builtin;
        struct Trace {value:Integer}
        impl Trace {
            comptime fn input(self)->meta.Object {self.value=self.value * 10 + 1;let object={flag:true,'x',flag:false};object}
            comptime fn bound(self,tag:Integer,value:Integer)->Integer {self.value=self.value * 10 + tag;value}
            comptime fn replacement(self)->meta.Object {self.value=self.value * 10 + 4;let object={flag:'y'};object}
        }
        comptime fn early()->bool {
            let input={flag:true};input.slice(do {return true;0},-1);false
        }
        comptime fn run()->bool {
            var trace=Trace {value:0};
            let changed=trace.input().replace_range(trace.bound(2,1),trace.bound(3,2),trace.replacement());
            trace.value==1234 and changed.length()==3
                and (changed.value<char>(1) matches 'y'?) and early()
        }").unwrap(),Value::Bool(true));
}

fn invoke_entries(source: &str) -> Result<Value, String> {
    let meta = include!("support/meta_source.rs");
    let package = compile(&format!("mod meta {{ {meta} }} {source}"))?;
    luna::invocation::invoke(&package, "run", 1_000_000)
}

#[test]
fn entry_combinators_preserve_domains_order_and_duplicate_keys() {
    let result = invoke_entries(
        "typealias Integer=.Integer'builtin;typealias Type=.Type'builtin;
        typealias char=.char'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn run()->bool {
            let absent:?bool=null;
            let input:meta.Object={key:true,'x',key:false,empty:absent,kind:Identity<bool>};
            input.foreach(|entry| {entry.value<bool>(0);});
            let seen=input.fold(0,|sum,entry|sum * 10 + entry.length());
            let filtered=input.filter(|entry| entry.EntryType(0)==bool);
            let flattened=input.flat_map(|entry| {
                if entry.EntryType(0)==char { ({}) } else { entry.concat(entry) }
            });
            let joined=input.fold({},|acc,entry|acc.concat(entry));
            seen==11111 and filtered.length()==2
                and (filtered.value<bool>(0) matches true?)
                and (filtered.get<bool>(.key) matches false?)
                and flattened.length()==8
                and (flattened.value<?bool>(4) match {v?=>v matches null,null=>false})
                and (flattened.get<Type>(.kind) match {T?=>T==Identity<bool>,null=>false})
                and joined.length()==5 and (joined.key(1) matches null)
                and (joined.value<char>(1) matches 'x'?)
                and input.length()==5
        }",
    )
    .unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn entry_combinators_keep_snapshot_and_private_callback_state() {
    let result = invoke(
        "typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            var input:meta.Object={first:true,second:false,third:true};
            var visits:Integer=0;
            let filtered=input.filter(|entry| {
                visits = visits + 1;
                entry.set(0,false);
                visits != 2
            });
            let expanded=filtered.flat_map(|entry| {
                visits = visits + 1;
                ({before:visits}).concat(entry)
            });
            let total=expanded.fold(0,|sum,entry|sum + entry.length());
            visits==0 and total==4 and input.length()==3
                and filtered.length()==2 and expanded.length()==4
                and (filtered.value<bool>(0) matches true?)
                and (filtered.value<bool>(1) matches true?)
                and (expanded.value<Integer>(0) matches 1?)
                and (expanded.value<Integer>(2) matches 2?)
                and (expanded.key(1) matches .first?)
                and (expanded.key(3) matches .third?)
        }",
    )
    .unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn empty_entry_combinators_skip_callbacks_and_preserve_fold_seed() {
    assert_eq!(
        invoke(
            "typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            let empty:meta.Object={};
            var count:Integer=0;
            empty.foreach(|entry| {entry.value<bool>(0);});
            let filtered=empty.filter(|entry| entry.value<bool>(0) matches true?);
            let mapped=empty.flat_map(|entry| entry.slice(0,1));
            let seed:?bool=null;
            let result=empty.fold(seed,|acc,entry| {entry.value<bool>(0);acc});
            count==0 and filtered.length()==0 and mapped.length()==0
                and (result matches null)
        }"
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn entry_combinators_propagate_callback_failure_and_reject_runtime_use() {
    for expression in [
        "entries.foreach(|entry| {entry.value<bool>(1);})",
        "entries.filter(|entry| entry.value<bool>(1) matches true?)",
        "entries.flat_map(|entry| entry.slice(0,2))",
        "entries.fold(true,|acc,entry| entry.value<bool>(1) matches true?)",
    ] {
        let result = invoke(&format!(
            "comptime fn run()->bool {{let entries:meta.Object={{flag:true}};{expression};true}}"
        ));
        let error = result.unwrap_err();
        assert!(error.contains("IndexOutOfBounds"), "{expression}: {error}");
    }
    let meta = include!("support/meta_source.rs");
    assert!(compile(&format!("mod meta {{{meta}}} fn bad()->meta.Object {{let data:meta.Object={{}};data.filter(|entry|true)}}")).is_err());
}

#[test]
fn foreach_calls_once_per_entry_and_vm_recovers_after_callback_failure() {
    let meta = include!("support/meta_source.rs");
    let package = compile(&format!(
        "mod meta {{{meta}}}
        comptime fn visit(entry:meta.Object) {{entry.value<bool>(0);}}
        comptime fn fail()->bool {{
            let data:meta.Object={{a:true,b:false}};
            data.foreach(|entry| {{entry.value<bool>(1);}});
            true
        }}
        comptime fn run()->bool {{
            let data:meta.Object={{a:true,false,a:false}};
            data.foreach(visit);
            true
        }}"
    ))
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "fail"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::IndexOutOfBounds
    );
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "visit")
            .count(),
        3
    );
}

#[test]
fn fallible_entry_traversal_preserves_errors_and_rejects_invalid_callbacks() {
    let source = include_str!("../../../development/examples/sysroot-meta-object-fallible.fl");
    assert_eq!(
        invoke_entries(&format!(
            "typealias Object=meta.Object; typealias Integer=meta.builtin.Integer;
             typealias Type=meta.builtin.Type; typealias char=meta.builtin.char;
             typealias void=meta.builtin.void; {source}"
        ))
        .unwrap(),
        Value::Bool(true)
    );
    let meta = include!("support/meta_source.rs");
    for source in [
        "fn bad(input:meta.Object)->!bool meta.Object {input.try_flat_map(callback)} comptime fn callback(entry:meta.Object)->!bool meta.Object {entry}",
        "comptime fn bad(input:meta.Object)->!bool meta.Object {input.try_flat_map(callback)} comptime fn callback(entry:bool)->!bool meta.Object { ({}) }",
        "comptime fn bad(input:meta.Object)->!bool bool {input.try_fold(true,callback)} comptime fn callback(acc:bool,entry:meta.Object)->!bool meta.Object {entry}",
        "comptime fn bad(input:meta.Object)->!bool meta.builtin.void {input.try_foreach(callback)} comptime fn callback(entry:meta.Object)->!bool bool {true}",
    ] {
        assert!(
            compile(&format!("mod meta {{{meta}}} {source}")).is_err(),
            "{source}"
        );
    }
}
