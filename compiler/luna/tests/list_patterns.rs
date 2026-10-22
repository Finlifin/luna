use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod support;
use support::{compile_with_source_bindings, id};

const PROTOCOL: &str = "typealias Type=.Type'builtin; typealias Size=.u64'builtin;
    trait ListView {
        assoc Element:Type;
        fn list_pattern_view_get_len(*self)->Size;
        fn list_pattern_view_get_element(*self,comptime index:Size)->Element;
    }
    struct Repeated {length:Size,value:bool}
    fn length() {} fn element() {}
    impl ListView for Repeated {
        assoc Element:Type=bool;
        fn list_pattern_view_get_len(*self)->Size {length();self.length.*}
        fn list_pattern_view_get_element(*self,comptime index:Size)->bool {
            element();self.value.*
        }
    }";

fn compile(body: &str) -> nir::program::Package {
    compile_with_source_bindings(
        &format!("{PROTOCOL} {body}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input::ListView"),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn pure_list_patterns_cannot_hide_extractor_writes() {
    let source = PROTOCOL
        .replace("length();self.length.*", "2")
        .replace(
            "struct Repeated {length:Size,value:bool}",
            "struct Repeated {length:Size,value:bool,flag:*bool}",
        )
        .replace("element();self.value.*", "self.flag.*.* = true;true");
    let result = compile_with_source_bindings(
        &format!(
            "{source}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        pure fn inspect(value:Repeated)->bool {{value match {{[_,_]=>true,_=>false}}}}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input::ListView"),
            copy: Some("Duplicate"),
            drop: Some("Retire"),
            ..Default::default()
        },
    );
    let error = result.unwrap_err();
    assert!(error.contains("P4"), "{error}");
}

#[test]
fn pure_list_patterns_accept_independently_pure_extractors() {
    let source = PROTOCOL
        .replace("length();self.length.*", "2")
        .replace("element();self.value.*", "true");
    let package=compile_with_source_bindings(&format!("{source}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        pure fn inspect(value:Repeated)->bool {{value match {{[false,_]=>false,[a,b]=>a and b,_=>false}}}}
        fn entry()->bool {{inspect(Repeated {{length:2,value:true}})}}"),
        &Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input::ListView"),copy:Some("Duplicate"),drop:Some("Retire"),..Default::default()}).unwrap();
    let graph = luna::instantiation::instantiate(&package, "entry", 200_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(
            &iir::InstanceKey {
                declaration: id(&package, "entry"),
                arguments: vec![]
            },
            vec![]
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_list_views_preserve_associated_results_across_calls() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn first(value:T,fallback:T.as(ListView).Element)->T.as(ListView).Element
            where T :- ListView {
            value match {[a,_]=>a,_=>fallback}
        }
        fn forward(value:T,fallback:T.as(ListView).Element)->T.as(ListView).Element
            where T :- ListView {first(value,fallback)}
        struct Indexed {length:Size}
        impl ListView for Indexed {
            assoc Element:Type=Size;
            fn list_pattern_view_get_len(*self)->Size {self.length.*}
            fn list_pattern_view_get_element(*self,comptime index:Size)->Size {index}
        }
        fn entry()->(bool,bool,Size,Size) {
            (forward(Repeated {length:2,value:true},false),
             forward(Repeated {length:1,value:false},true),
             forward(Indexed {length:2},9),forward(Indexed {length:1},9))
        }",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            200_000
        )
        .unwrap(),
        Value::Tuple(vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::UnsignedInteger(0),
            Value::UnsignedInteger(9)
        ])
    );
}

#[test]
fn list_view_patterns_check_length_and_extract_in_order() {
    let package=compile("fn inspect(value:Repeated)->bool {value match {
        [a,(b if a == b)]=>a,_=>false,
    }} fn run(size:Size)->(bool,bool) {(inspect(Repeated {length:size,value:true}),inspect(Repeated {length:size,value:false}))}");
    let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(
            &iir::InstanceKey {
                declaration: id(&package, "run"),
                arguments: vec![]
            },
            vec![Value::UnsignedInteger(2)]
        )
        .unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::Bool(false)])
    );
    let trace = names(&graph, &vm)
        .into_iter()
        .filter(|name| matches!(name.as_str(), "length" | "element"))
        .collect::<Vec<_>>();
    assert_eq!(
        trace,
        [
            "length", "element", "element", "length", "element", "element"
        ]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn list_view_patterns_skip_unreachable_extractors_and_retry_candidates() {
    let package = compile(
        "fn run(size:Size)->bool {Repeated {length:size,value:true} match {
        []=>false,[true]=>false,[false,_]=>false,[a,true]=>a,_=>false,
    }}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(
            &iir::InstanceKey {
                declaration: id(&package, "run"),
                arguments: vec![]
            },
            vec![Value::UnsignedInteger(2)]
        )
        .unwrap(),
        Value::Bool(true)
    );
    let trace = names(&graph, &vm)
        .into_iter()
        .filter(|name| matches!(name.as_str(), "length" | "element"))
        .collect::<Vec<_>>();
    assert_eq!(
        trace,
        [
            "length", "length", "length", "element", "length", "element", "element"
        ]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

fn names(graph: &iir::Package, vm: &Interpreter<'_>) -> Vec<String> {
    vm.events()
        .iter()
        .filter_map(|event| {
            let comptime_interpretor::Event::EnterFunction(id) = event else {
                return None;
            };
            graph
                .functions
                .iter()
                .find(|function| function.key.declaration == *id)
                .map(|function| function.function.name.clone())
        })
        .collect()
}

#[test]
fn list_view_indices_are_staged_and_resource_checked() {
    let source = PROTOCOL
        .replace("assoc Element:Type=bool", "assoc Element:Type=Size")
        .replace("comptime index:Size)->bool", "comptime index:Size)->Size")
        .replace("element();self.value.*", "element();index");
    let package=compile_with_source_bindings(&format!("{source}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        fn entry()->(Size,Size) {{Repeated {{length:2,value:true}} match {{[a,b]=>(a,b),_=>(9,9)}}}}"),
        &Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input::ListView"),..Default::default()}).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            200_000
        )
        .unwrap(),
        Value::Tuple(vec![Value::UnsignedInteger(0), Value::UnsignedInteger(1)])
    );
}

#[test]
fn list_view_candidates_do_not_retire_subject_on_failure() {
    let source = PROTOCOL.replace(
        "struct Repeated {length:Size,value:bool}",
        "struct Repeated {length:Size,value:bool,retired:*bool,duplicate:*bool}",
    );
    for arms in [
        "[false,_]=>(),[a,(b if a == b)]=>()",
        "([a,b] if not a) or [b,a]=>()",
        "([a,b] if not a) or ([b,a] if not b)=>()",
    ] {
        let package=compile_with_source_bindings(&format!("{source}
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Retire for Repeated {{fn drop(*self) {{
                if self.retired.*.* {{self.duplicate.*.* = true;}}
                self.retired.*.* = true;
            }}}}
            fn entry()->bool {{
                var retired=false;var duplicate=false;
                {{let subject=Repeated {{length:2,value:true,retired:retired.ref,duplicate:duplicate.ref}};
                  subject match {{{arms},_=>()}};}}
                retired and not duplicate
            }}"),&Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input::ListView"),..Default::default()}).unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "entry",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap(),
            Value::Bool(true),
            "{arms}"
        );
    }
}

#[test]
fn list_view_rejects_missing_or_malformed_protocols() {
    let failure = support::compile_with_protocols(
        &format!("{PROTOCOL} fn run(value:Repeated)->bool {{value match {{[a,b]=>a,_=>false}}}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("bound ListPatternView"), "{failure}");
    for source in [
        PROTOCOL.replace("comptime index:Size", "index:Size"),
        PROTOCOL.replace("get_len(*self)", "get_len(self)"),
        PROTOCOL.replace("comptime index:Size", "comptime index:.u32'builtin"),
    ] {
        let failure = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input::ListView"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(failure.contains("ListPatternView"), "{failure}");
    }
}

#[test]
fn list_view_candidates_preserve_let_else_scope() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn fallback()->bool {
            let a=true;
            let [a,(_ if a)]=Repeated {length:2,value:false} else {return a;};
            false
        }
        fn entry()->bool {fallback()}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            200_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_view_observation_rejects_mutation_and_rejection() {
    for replacement in [
        "self.value.* = false;element();self.value.*",
        "assumes false;element();self.value.*",
    ] {
        let source = PROTOCOL.replace("element();self.value.*", replacement);
        let package = compile_with_source_bindings(
            &format!(
                "{source}
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            fn entry()->bool {{Repeated {{length:2,value:true}} match {{[a,b]=>a,_=>false}}}}"
            ),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input::ListView"),
                ..Default::default()
            },
        )
        .unwrap();
        let failure = luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            failure.contains("guard pointer observation")
                || failure.contains("rejecting guard call"),
            "{failure}"
        );
    }
}

#[test]
fn list_view_scope_and_phase_failures_remain_explicit() {
    for (body, expected) in [
        (
            "fn bad(value:Repeated)->bool {value match {[(_ if b),b]=>true,_=>false}}",
            "HIR lowering",
        ),
        (
            "fn bad(value:Repeated)->bool {value match {[a,a]=>true,_=>false}}",
            "duplicate pattern binding",
        ),
        (
            "comptime fn bad(value:Repeated)->bool {value match {[a,b]=>a,_=>false}}",
            "comptime ListPatternView phase",
        ),
    ] {
        let failure = compile_with_source_bindings(
            &format!("{PROTOCOL} {body}"),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input::ListView"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(failure.contains(expected), "{failure}");
    }
}

#[test]
fn list_view_alternatives_keep_internal_subject_bindings_private() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn entry()->bool {Repeated {length:2,value:true} match {
            ([a,b] if not a) or [b,a]=>a and b,_=>false,
        }}",
    );
    let value = luna::execution::interpret_with_protocols(
        &package,
        "entry",
        "Duplicate",
        "Retire",
        200_000,
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn list_view_scoped_observation_composes_with_whole_and_nested_bindings() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn whole(value:Repeated)->bool {value match {
            (([a,b] if not a) or [b,a]) as all=>all.value and a and b,_=>false,
        }}
        fn nested(value:(Repeated,Repeated))->bool {value match {
            (([a,b] if not a) or [b,a],[c,d])=>a and b and c and d,_=>false,
        }}
        fn entry()->bool {
            whole(Repeated {length:2,value:true}) and
            nested((Repeated {length:2,value:true},Repeated {length:2,value:true}))
        }",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            300_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_view_scoped_subjects_remain_hygienic_under_inline_and_quote_expansion() {
    for modifier in ["inline", "quote"] {
        let package=compile(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            {modifier} fn inspect(value:Repeated)->bool {{value match {{[a,b]=>a and b,_=>false}}}}
            fn entry()->bool {{inspect(Repeated {{length:2,value:true}}) and not inspect(Repeated {{length:2,value:false}})}}"));
        let result = if modifier == "inline" {
            luna::execution::interpret_inline_with_protocols_in_package(
                &package,
                0,
                "entry",
                "Duplicate",
                "Retire",
                300_000,
            )
        } else {
            luna::execution::interpret_with_protocols(
                &package,
                "entry",
                "Duplicate",
                "Retire",
                300_000,
            )
        };
        assert_eq!(result.unwrap(), Value::Bool(true), "{modifier}");
    }
}

#[test]
fn observation_subject_addresses_cannot_escape_into_successful_bindings() {
    use nir::program::{Expr, ExprKind as K, LocalId, Pattern};
    let mut package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn entry()->bool {Repeated {length:2,value:true} match {[a,b]=>true,_=>false}}",
    );
    let function = package
        .functions
        .iter_mut()
        .find(|function| function.name == "entry")
        .unwrap();
    let K::Block {
        result: Some(result),
        ..
    } = &mut function.body.kind
    else {
        panic!("entry block")
    };
    let K::Match { value, arms } = &mut result.kind else {
        panic!("entry match")
    };
    let Pattern::Observe { binding, inner } = &mut arms[0].pattern else {
        panic!("observation scope")
    };
    **inner = Pattern::Then {
        first: Box::new(Pattern::Wild),
        value: Box::new(Expr {
            source: result.source,
            ty: nir::Term::Pointer(Box::new(value.ty.clone())),
            kind: K::AddressOf(Box::new(Expr {
                source: result.source,
                ty: value.ty.clone(),
                kind: K::Local(*binding),
            })),
        }),
        next: Box::new(Pattern::Binding {
            binding: LocalId {
                owner: u32::MAX,
                index: 10000,
            },
            inner: Box::new(Pattern::Wild),
        }),
    };
    let failure = luna::execution::interpret_with_protocols(
        &package,
        "entry",
        "Duplicate",
        "Retire",
        200_000,
    )
    .unwrap_err();
    assert!(
        failure.contains("observation subject address escapes"),
        "{failure}"
    );
}

#[test]
fn quote_list_extractors_preserve_candidate_order_and_staged_indices() {
    let source = PROTOCOL
        .replace(
            "fn list_pattern_view_get_len(*self)->Size {",
            "quote fn list_pattern_view_get_len(*self)->Size {",
        )
        .replace(
            "fn list_pattern_view_get_element(*self,comptime index:Size)->bool {",
            "quote fn list_pattern_view_get_element(*self,comptime index:Size)->bool {",
        );
    let package=compile_with_source_bindings(&format!("{source}
        fn entry()->bool {{Repeated {{length:2,value:true}} match {{[false,_]=>false,[a,b]=>a and b,_=>false}}}}"),
        &Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input::ListView"),..Default::default()}).unwrap();
    let graph = luna::instantiation::instantiate(&package, "entry", 200_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let observed = names(&graph, &vm);
    let trace: Vec<_> = observed
        .iter()
        .filter(|name| matches!(name.as_str(), "length" | "element"))
        .collect();
    assert_eq!(
        trace,
        vec!["length", "element", "length", "element", "element"]
    );
    assert!(
        observed.iter().any(|name| name.ends_with("::callable")),
        "{observed:?}"
    );
}

#[test]
fn quote_list_extractors_remain_resource_checked() {
    let source = PROTOCOL
        .replace(
            "fn list_pattern_view_get_len(*self)->Size {",
            "quote fn list_pattern_view_get_len(*self)->Size {",
        )
        .replace(
            "fn list_pattern_view_get_element(*self,comptime index:Size)->bool {",
            "quote fn list_pattern_view_get_element(*self,comptime index:Size)->bool {",
        );
    let package = compile_with_source_bindings(
        &format!(
            "{source}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        fn entry()->bool {{Repeated {{length:2,value:true}} match {{[a,b]=>a and b,_=>false}}}}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input::ListView"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            200_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace("element();self.value.*", "self.value.* = false;false");
    let invalid = compile_with_source_bindings(
        &format!(
            "{invalid}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        fn entry()->bool {{Repeated {{length:2,value:true}} match {{[a,b]=>a and b,_=>false}}}}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input::ListView"),
            ..Default::default()
        },
    )
    .unwrap();
    let failure = luna::execution::interpret_with_protocols(
        &invalid,
        "entry",
        "Duplicate",
        "Retire",
        200_000,
    )
    .unwrap_err();
    assert!(failure.contains("observation"), "{failure}");
}
