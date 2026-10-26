use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod support;
use support::{compile_with_tuple_selectors as compile_with_source_bindings, id};

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

#[test]
fn list_wildcard_rest_checks_minimum_length_and_only_extracts_prefix() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn inspect(value:Repeated)->bool {value match {
            [a,b if a==b,..._]=>a,_=>false,
        }}
        fn run(size:Size)->bool {inspect(Repeated {length:size,value:true})}
        fn entry()->bool {not run(0) and not run(1) and run(2) and run(9)}",
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
    let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    for (length, expected, elements) in [(0, false, 0), (1, false, 0), (2, true, 2), (9, true, 2)] {
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![Value::UnsignedInteger(length)])
                .unwrap(),
            Value::Bool(expected)
        );
        let calls = names(&graph, &vm);
        assert_eq!(
            calls
                .iter()
                .filter(|name| name.as_str() == "length")
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|name| name.as_str() == "element")
                .count(),
            elements
        );
    }
}

#[test]
fn list_rest_must_be_last_and_named_rest_requires_a_protocol() {
    for (pattern, message) in [
        ("[..._,a]", "list rest pattern must occur once at the end"),
        (
            "[..._,..._]",
            "list rest pattern must occur once at the end",
        ),
        (
            "[a,...tail]",
            "list rest patterns require a slicing protocol",
        ),
    ] {
        let error = compile_with_source_bindings(&format!("{PROTOCOL} fn inspect(value:Repeated)->bool {{value match {{{pattern}=>true,_=>false}}}}"),
            &Default::default(), luna::protocols::type_constructors::Selectors {
                list_pattern:Some("input.ListView"),..Default::default()
            }).unwrap_err();
        assert!(error.contains(message), "{pattern}: {error}");
    }
}

#[test]
fn wildcard_rest_empty_prefix_and_failed_candidates_preserve_observation_order() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn inspect(value:Repeated)->bool {value match {
            [false,_,..._]=>false,
            [a,..._] if false=>a,
            [..._]=>true,
            _=>false,
        }}
        fn run()->bool {inspect(Repeated {length:9,value:true})}
        fn empty()->bool {inspect(Repeated {length:0,value:true})}",
    );
    for (entry, expected_calls) in [
        (
            "run",
            vec!["length", "element", "length", "element", "length"],
        ),
        ("empty", vec!["length", "length", "length"]),
    ] {
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                entry,
                "Duplicate",
                "Retire",
                300_000
            )
            .unwrap(),
            Value::Bool(true)
        );
        let graph = luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        let calls = names(&graph, &vm)
            .into_iter()
            .filter(|name| matches!(name.as_str(), "length" | "element"))
            .collect::<Vec<_>>();
        assert_eq!(calls, expected_calls);
    }
}

#[test]
fn comptime_list_wildcard_rest_uses_the_same_prefix_rule() {
    let protocol = PROTOCOL
        .replace("fn list_pattern_view", "comptime fn list_pattern_view")
        .replace("length();self.length.*", "self.length.*")
        .replace("element();self.value.*", "self.value.*");
    let source = format!(
        "{protocol}
        comptime fn inspect(value:Repeated)->bool {{value match {{[a,b,..._]=>a and b,_=>false}}}}
        comptime fn result()->bool {{
            not inspect(Repeated {{length:1,value:true}})
            and inspect(Repeated {{length:2,value:true}})
            and inspect(Repeated {{length:9,value:true}})
            and not inspect(Repeated {{length:9,value:false}})
        }}"
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input.ListView"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "result", 300_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn quoted_list_wildcard_rest_preserves_exported_prefix_bindings() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        comptime fn prefix()-><Repeated -| first:bool,second:bool> {'<[first,second,..._]>}
        fn inspect(value:Repeated)->bool {
            const code=prefix();
            value match {~[first=a,second=b]<code> => a and b,_=>false}
        }
        fn entry()->bool {
            inspect(Repeated {length:2,value:true})
            and inspect(Repeated {length:9,value:true})
            and not inspect(Repeated {length:1,value:true})
            and not inspect(Repeated {length:9,value:false})
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
fn negated_list_views_invert_prefix_matches_with_copy_evidence() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for Repeated {}
        fn inspect(value:Repeated)->bool {value match {
            not [true,..._] => true,
            _=>false,
        }}
        fn entry()->bool {
            inspect(Repeated {length:0,value:true})
            and inspect(Repeated {length:2,value:false})
            and not inspect(Repeated {length:2,value:true})
        }";
    let selectors = luna::protocols::type_constructors::Selectors {
        list_pattern: Some("input.ListView"),
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let run = |body: &str| {
        let package = compile_with_source_bindings(
            &format!("{PROTOCOL}{body}"),
            &Default::default(),
            selectors,
        )
        .unwrap();
        luna::execution::interpret_with_protocols(&package, "entry", "Duplicate", "Retire", 300_000)
    };
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let error = run(&source.replace("impl Duplicate for Repeated {}", "")).unwrap_err();
    assert!(
        error.contains("Copy") || error.contains("transfer"),
        "{error}"
    );
}

fn compile(body: &str) -> nir::program::Package {
    compile_with_source_bindings(
        &format!("{PROTOCOL} {body}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input.ListView"),
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
            list_pattern: Some("input.ListView"),
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
        &Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input.ListView"),copy:Some("Duplicate"),drop:Some("Retire"),..Default::default()}).unwrap();
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
        fn entry()->Tuple<bool,bool,Size,Size> {
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
    }} fn run(size:Size)->Tuple<bool,bool> {(inspect(Repeated {length:size,value:true}),inspect(Repeated {length:size,value:false}))}");
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
        fn entry()->Tuple<Size,Size> {{Repeated {{length:2,value:true}} match {{[a,b]=>(a,b),_=>(9,9)}}}}"),
        &Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input.ListView"),..Default::default()}).unwrap();
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
            }}"),&Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input.ListView"),..Default::default()}).unwrap();
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
                list_pattern: Some("input.ListView"),
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
                list_pattern: Some("input.ListView"),
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
                list_pattern: Some("input.ListView"),
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
        fn nested(value:Tuple<Repeated,Repeated>)->bool {value match {
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
        &Default::default(),luna::protocols::type_constructors::Selectors {list_pattern:Some("input.ListView"),..Default::default()}).unwrap();
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
        observed.iter().any(|name| name.ends_with(".callable")),
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
            list_pattern: Some("input.ListView"),
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
            list_pattern: Some("input.ListView"),
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

#[test]
fn list_pattern_arity_respects_the_bound_size_representation() {
    let protocol = PROTOCOL.replace(".u64'builtin", ".u8'builtin");
    for count in [0usize, 1, 255, 256] {
        let elements = std::iter::repeat_n("_", count)
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            "{protocol} fn run(value:Repeated)->bool {{value match {{[{elements}]=>true,_=>false}}}}"
        );
        let result = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input.ListView"),
                ..Default::default()
            },
        );
        if count <= 255 {
            assert!(result.is_ok(), "count={count}: {:?}", result.err());
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("range") || error.contains("fit"), "{error}");
        }
    }
}

#[test]
fn list_pattern_protocol_validation_precedes_any_pattern_use() {
    for source in [
        PROTOCOL.replace("assoc Element:Type;", "assoc Element:bool;"),
        PROTOCOL.replace("get_len(*self)->Size;", "get_len(*self)->bool;"),
        PROTOCOL.replace(
            "get_element(*self,comptime index:Size)->Element;",
            "get_element(*self,comptime index:Size)->bool;",
        ),
        PROTOCOL.replace(
            "fn list_pattern_view_get_len(*self)->Size;",
            "comptime fn list_pattern_view_get_len(*self)->Size;",
        ),
    ] {
        // The entry itself has no list pattern. Binding an invalid language
        // protocol must not succeed merely because no caller uses it yet.
        let declaration = source.split("struct Repeated").next().unwrap();
        let source = format!("{declaration} fn entry()->bool {{true}}");
        let error = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input.ListView"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("ListPatternView") || error.contains("associated value used as a type"),
            "{error}"
        );
    }
}

#[test]
fn long_flat_list_patterns_execute_with_bounded_ordered_observation() {
    let elements = std::iter::repeat_n("true", 255)
        .collect::<Vec<_>>()
        .join(",");
    let package = compile(&format!(
        "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
         fn inspect(value:Repeated)->bool {{value match {{[{elements}]=>true,_=>false}}}}
         fn entry()->bool {{inspect(Repeated{{length:255,value:true}})}}"
    ));
    let graph = luna::instantiation::instantiate(&package, "inspect", 1_000_000).unwrap();
    for (length, value, expected, extracted) in [
        (255, true, true, 255),
        (255, false, false, 1),
        (254, true, false, 0),
    ] {
        let repeated = package
            .structs
            .iter()
            .find(|s| s.name == "Repeated")
            .unwrap()
            .declaration;
        let mut vm = Interpreter::from_iir(
            &graph,
            Budget {
                steps: 1_000_000,
                depth: 128,
            },
        )
        .unwrap();
        let result = vm
            .invoke_instance(
                &iir::InstanceKey {
                    declaration: id(&package, "inspect"),
                    arguments: vec![],
                },
                vec![Value::Record {
                    declaration: repeated,
                    arguments: vec![],
                    fields: vec![Value::UnsignedInteger(length), Value::Bool(value)],
                }],
            )
            .unwrap();
        assert_eq!(result, Value::Bool(expected));
        let trace = names(&graph, &vm);
        assert_eq!(
            trace
                .iter()
                .filter(|name| name.as_str() == "length")
                .count(),
            1
        );
        assert_eq!(
            trace
                .iter()
                .filter(|name| name.as_str() == "element")
                .count(),
            extracted
        );
    }
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn long_list_alternatives_preserve_cross_group_bindings_and_retry_order() {
    let middle = std::iter::repeat_n("_", 253).collect::<Vec<_>>().join(",");
    let package = compile(&format!(
        "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
         fn inspect(value:Repeated)->bool {{value match {{
             [first,{middle},(last if not first)] or [first,{middle},(last if first)] => first and last,
             _ => false
         }}}}
         fn entry()->bool {{inspect(Repeated{{length:255,value:true}})}}"
    ));
    let graph = luna::instantiation::instantiate(&package, "inspect", 1_000_000).unwrap();
    let repeated = package
        .structs
        .iter()
        .find(|s| s.name == "Repeated")
        .unwrap()
        .declaration;
    for (value, attempts) in [(true, 2), (false, 1)] {
        let mut vm = Interpreter::from_iir(
            &graph,
            Budget {
                steps: 1_000_000,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke_instance(
                &iir::InstanceKey {
                    declaration: id(&package, "inspect"),
                    arguments: vec![],
                },
                vec![Value::Record {
                    declaration: repeated,
                    arguments: vec![],
                    fields: vec![Value::UnsignedInteger(255), Value::Bool(value)]
                }]
            )
            .unwrap(),
            Value::Bool(value)
        );
        let trace = names(&graph, &vm)
            .into_iter()
            .filter(|name| matches!(name.as_str(), "length" | "element"))
            .collect::<Vec<_>>();
        let expected = (0..attempts)
            .flat_map(|_| std::iter::once("length").chain(std::iter::repeat_n("element", 255)))
            .collect::<Vec<_>>();
        assert_eq!(trace, expected);
    }
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_list_views_execute_and_select_runtime_types() {
    let protocol = PROTOCOL
        .replace("fn list_pattern_view", "comptime fn list_pattern_view")
        .replace("length();self.length.*", "self.length.*")
        .replace("element();self.value.*", "self.value.*");
    let package = compile_with_source_bindings(
        &format!(
            "{protocol}
            comptime fn inspect(value:Repeated)->bool {{
                value match {{[a,(b if a == b)]=>a,_=>false}}
            }}
            comptime fn first(value:T,fallback:T.as(ListView).Element)->T.as(ListView).Element
                where T :- ListView {{value match {{[a,_]=>a,_=>fallback}}}}
            comptime fn Result()->Type {{
                if inspect(Repeated{{length:2,value:true}}) {{bool}} else {{.u8'builtin}}
            }}
            typealias Chosen=Result();fn entry()->Chosen {{true}}
            comptime fn result()->bool {{
                inspect(Repeated{{length:2,value:true}})
                and not inspect(Repeated{{length:1,value:true}})
                and not inspect(Repeated{{length:2,value:false}})
                and first(Repeated{{length:2,value:true}},false)
                and first(Repeated{{length:0,value:false}},true)
            }}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input.ListView"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "result", 200_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "entry", 200_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_list_views_reject_runtime_use_and_mixed_method_phases() {
    let protocol = PROTOCOL
        .replace("fn list_pattern_view", "comptime fn list_pattern_view")
        .replace("length();self.length.*", "self.length.*")
        .replace("element();self.value.*", "self.value.*");
    for source in [
        format!("{protocol}fn bad(value:Repeated)->bool {{value match {{[]=>true,_=>false}}}}"),
        protocol.replace(
            "comptime fn list_pattern_view_get_element",
            "fn list_pattern_view_get_element",
        ),
    ] {
        let error = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input.ListView"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("ListPatternView"), "{error}");
    }
}

#[test]
fn comptime_list_views_short_circuit_before_failing_extractors() {
    let protocol = PROTOCOL
        .replace("fn list_pattern_view", "comptime fn list_pattern_view")
        .replace("length();self.length.*", "self.length.*")
        .replace(
            "element();self.value.*",
            "if index == 0 {self.value.*} else {while true {} false}",
        );
    let package = compile_with_source_bindings(
        &format!(
            "{protocol}
            comptime fn first(value:Repeated)->bool {{
                value match {{[true,_]=>false,_=>true}}
            }}
            comptime fn exhaust()->bool {{first(Repeated{{length:2,value:true}})}}
            comptime fn empty(value:Repeated)->bool {{value match {{[]=>true,_=>false}}}}
            comptime fn result()->bool {{
                first(Repeated{{length:1,value:true}})
                and first(Repeated{{length:2,value:false}})
                and empty(Repeated{{length:0,value:true}})
            }}
            const ANSWER=result();
            comptime fn saved()->bool {{ANSWER}}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input.ListView"),
            ..Default::default()
        },
    )
    .unwrap();
    for entry in ["result", "saved"] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    let failure = luna::invocation::invoke(&package, "exhaust", 10_000).unwrap_err();
    assert!(
        failure.contains("Budget") || failure.contains("budget"),
        "{failure}"
    );
    assert_eq!(
        luna::invocation::invoke(&package, "result", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_list_views_can_extract_types_without_runtime_splicing() {
    let protocol = PROTOCOL
        .replace("fn list_pattern_view", "comptime fn list_pattern_view")
        .replace("Element:Type=bool", "Element:Type=Type")
        .replace("index:Size)->bool", "index:Size)->Type")
        .replace("length();self.length.*", "self.length.*")
        .replace(
            "element();self.value.*",
            "if self.value.* {bool} else {.u8'builtin}",
        );
    let package = compile_with_source_bindings(
        &format!(
            "{protocol}
            comptime fn First(value:T,fallback:T.as(ListView).Element)->T.as(ListView).Element
                where T :- ListView {{value match {{[a,_]=>a,_=>fallback}}}}
            typealias Chosen=First(Repeated{{length:2,value:true}},.u8'builtin);
            typealias Fallback=First(Repeated{{length:0,value:false}},bool);
            fn entry()->Chosen {{true}} fn fallback()->Fallback {{true}}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            list_pattern: Some("input.ListView"),
            ..Default::default()
        },
    )
    .unwrap();
    for entry in ["entry", "fallback"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn comptime_list_views_read_stored_type_elements_with_copy_evidence() {
    let protocol = PROTOCOL
        .replace("fn list_pattern_view", "comptime fn list_pattern_view")
        .replace("value:bool", "value:Type")
        .replace("Element:Type=bool", "Element:Type=Type")
        .replace("index:Size)->bool", "index:Size)->Type")
        .replace("length();self.length.*", "self.length.*")
        .replace("element();self.value.*", "self.value.*");
    let source = format!(
        "{protocol}
        trait Duplicate {{}}trait Retire {{fn drop(*self);}}
        impl Duplicate for Type {{}}
        comptime fn First(value:Repeated)->Type {{value match {{[a,_]=>a,_=>bool}}}}
        typealias Selected=First(Repeated{{length:2,value:bool}});
        fn entry()->Selected {{true}}"
    );
    let compile = |source: &str| {
        compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input.ListView"),
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
    };
    let package = compile(&source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "entry", 100_000).unwrap(),
        Value::Bool(true)
    );
    let missing = compile(&source.replace("impl Duplicate for Type {}", "")).unwrap_err();
    assert!(missing.contains("Copy"), "{missing}");
}

#[test]
fn generated_list_patterns_execute_in_the_quoted_runtime_phase() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        comptime fn pair_code()-><Repeated -| left:bool,right:bool> {'<[left,right]>}
        fn inspect(value:Repeated)->bool {
            const pair=pair_code();
            value match {~[left=a,right=b]<pair> if not a => false,~[left=x,right=y]<pair> => x and y,_=>false}
        }
        fn entry()->bool {inspect(Repeated{length:2,value:true}) and not inspect(Repeated{length:1,value:true}) and not inspect(Repeated{length:2,value:false})}");
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
fn quoted_list_patterns_reject_comptime_only_extractors() {
    let protocol = PROTOCOL
        .replace("fn list_pattern_view", "comptime fn list_pattern_view")
        .replace("length();self.length.*", "self.length.*")
        .replace("element();self.value.*", "self.value.*");
    let error=compile_with_source_bindings(
        &format!("{protocol}comptime fn pair_code()-><Repeated -| left:bool,right:bool> {{'<[left,right]>}}"),
        &Default::default(),luna::protocols::type_constructors::Selectors {
            list_pattern:Some("input.ListView"),..Default::default()
        }).unwrap_err();
    assert!(error.contains("runtime ListPatternView phase"), "{error}");
}

#[test]
fn list_views_extract_normal_form_tuples_with_ordered_nested_bindings() {
    let protocol = PROTOCOL
        .replace("Element:Type=bool", "Element:Type=Tuple<bool,bool>")
        .replace("index:Size)->bool", "index:Size)->Tuple<bool,bool>")
        .replace(
            "element();self.value.*",
            "element();(self.value.*,not self.value.*)",
        );
    let source = format!(
        "{protocol}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for Tuple<bool,bool> {{}}
        fn inspect(value:T)->bool where T :- ListView,
            requires T.as(ListView).Element == Tuple<bool,bool> {{
            value match {{
                [(a,b),(c if a == c,d if b == d)] => a and not b,
                _ => false,
            }}
        }}
        fn entry()->bool {{
            inspect(Repeated {{length:2,value:true}})
            and not inspect(Repeated {{length:2,value:false}})
            and not inspect(Repeated {{length:1,value:true}})
        }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        list_pattern: Some("input.ListView"),
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
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
    let without_copy = source.replace("impl Duplicate for Tuple<bool,bool> {}", "");
    let package =
        compile_with_source_bindings(&without_copy, &Default::default(), selectors).unwrap();
    let error = luna::execution::interpret_with_protocols(
        &package,
        "entry",
        "Duplicate",
        "Retire",
        300_000,
    )
    .unwrap_err();
    assert!(
        error.contains("computed pattern value needs independent resource"),
        "{error}"
    );
}

#[test]
fn list_views_extract_copy_records_and_retry_failed_candidates() {
    let protocol = PROTOCOL
        .replace("Element:Type=bool", "Element:Type=Item")
        .replace("index:Size)->bool", "index:Size)->Item")
        .replace(
            "element();self.value.*",
            "element();Item {first:self.value.*,second:not self.value.*}",
        );
    let source = format!(
        "{protocol}
        struct Item {{first:bool,second:bool}}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for Item {{}}
        fn inspect(value:Repeated)->bool {{value match {{
            [{{first:a}},{{second:b}}] if false => a and b,
            [{{first:a,second:b}},{{first:(c if a==c),second:(d if b==d)}}] => a and not b,
            _=>false,
        }}}}
        fn entry()->bool {{
            inspect(Repeated {{length:2,value:true}})
            and not inspect(Repeated {{length:2,value:false}})
            and not inspect(Repeated {{length:1,value:true}})
        }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        list_pattern: Some("input.ListView"),
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
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
    let without_copy = source.replace("impl Duplicate for Item {}", "");
    let package =
        compile_with_source_bindings(&without_copy, &Default::default(), selectors).unwrap();
    assert!(
        luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            300_000
        )
        .is_err()
    );
}

#[test]
fn list_view_nominal_copy_checks_nested_resources() {
    let protocol = PROTOCOL
        .replace("Element:Type=bool", "Element:Type=Item")
        .replace("index:Size)->bool", "index:Size)->Item")
        .replace(
            "element();self.value.*",
            "Item {field:Inner {value:self.value.*}}",
        );
    let source = format!(
        "{protocol}
        struct Inner {{value:bool}}
        struct Item {{field:Inner}}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for Inner {{}}
        impl Duplicate for Item {{}}
        fn inspect(value:Repeated)->bool {{value match {{
            [{{field:{{value:a}}}}] if false => a,
            [{{field:{{value:a}}}}] => a,
            _=>false,
        }}}}
        fn entry()->bool {{inspect(Repeated {{length:1,value:true}})}}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        list_pattern: Some("input.ListView"),
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let run = |source: &str| {
        let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
        luna::execution::interpret_with_protocols(&package, "entry", "Duplicate", "Retire", 300_000)
    };
    assert_eq!(run(&source).unwrap(), Value::Bool(true));
    let error = run(&source.replace("impl Duplicate for Inner {}", "")).unwrap_err();
    assert!(
        error.contains("Copy component has no Copy capability"),
        "{error}"
    );
    let error = run(&format!(
        "{source} impl Retire for Inner {{fn drop(*self) {{}}}}"
    ))
    .unwrap_err();
    assert!(
        error.contains("Copy") && error.contains("cleanup"),
        "{error}"
    );
}

#[test]
fn list_view_copy_record_does_not_hide_pointer_provenance() {
    let protocol = PROTOCOL
        .replace("Element:Type=bool", "Element:Type=Item")
        .replace("index:Size)->bool", "index:Size)->Item")
        .replace("element();self.value.*", "Item {field:self.value}");
    let source = format!(
        "{protocol}
        struct Item {{field:*bool}}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for Item {{}}
        fn inspect(value:Repeated)->bool {{value match {{[{{field:_}}]=>true,_=>false}}}}
        fn entry()->bool {{inspect(Repeated {{length:1,value:true}})}}"
    );
    for wrapped in [false, true] {
        let source = if wrapped {
            source
                .replace("Element:Type=Item", "Element:Type=Tuple<Item,bool>")
                .replace("index:Size)->Item", "index:Size)->Tuple<Item,bool>")
                .replace("Item {field:self.value}", "(Item {field:self.value},true)")
                .replace("[{field:_}]", "[({field:_},_)]")
                + " impl Duplicate for Tuple<Item,bool> {}"
        } else {
            source.clone()
        };
        let package = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                list_pattern: Some("input.ListView"),
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
        .unwrap();
        let error = luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            300_000,
        )
        .unwrap_err();
        assert!(
            error.contains("computed pattern value needs independent resource"),
            "{error}"
        );
    }
}

#[test]
fn list_view_enum_copy_checks_inactive_variants() {
    let protocol = PROTOCOL
        .replace("Element:Type=bool", "Element:Type=Item")
        .replace("index:Size)->bool", "index:Size)->Item")
        .replace("element();self.value.*", ".some(self.value.*)");
    let source = format!(
        "{protocol}
        struct Hidden {{value:bool}}
        enum Item {{some(bool),other(Hidden)}}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for Item {{}}
        impl Duplicate for Hidden {{}}
        fn inspect(value:Repeated)->bool {{value match {{
            [.some(a)] if false => a,
            [.some(a)] => a,
            _=>false,
        }}}}
        fn entry()->bool {{inspect(Repeated {{length:1,value:true}})}}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        list_pattern: Some("input.ListView"),
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    for hidden_copy in [true, false] {
        let source = if hidden_copy {
            source.clone()
        } else {
            source.replace("impl Duplicate for Hidden {}", "")
        };
        let package =
            compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
        let result = luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            300_000,
        );
        if hidden_copy {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("Copy component has no Copy capability"),
                "{error}"
            );
        }
    }
}

#[test]
fn list_view_normal_form_tuple_composes_with_nominal_elements() {
    let protocol = PROTOCOL
        .replace("Element:Type=bool", "Element:Type=Tuple<Item,bool>")
        .replace("index:Size)->bool", "index:Size)->Tuple<Item,bool>")
        .replace(
            "element();self.value.*",
            "(Item {value:self.value.*},not self.value.*)",
        );
    let source = format!(
        "{protocol}
        struct Item {{value:bool}}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for Item {{}}
        impl Duplicate for Tuple<Item,bool> {{}}
        fn inspect(value:Repeated)->bool {{value match {{
            [({{value:a}},b),_] if false => a and b,
            [({{value:a}},b),({{value:(c if a==c)}},d if b==d)] => a and not b,
            _=>false,
        }}}}
        fn entry()->bool {{
            inspect(Repeated {{length:2,value:true}})
            and not inspect(Repeated {{length:2,value:false}})
            and not inspect(Repeated {{length:1,value:true}})
        }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        list_pattern: Some("input.ListView"),
        copy: Some("Duplicate"),
        drop: Some("Retire"),
        ..Default::default()
    };
    let run = |source: &str| {
        let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
        luna::execution::interpret_with_protocols(&package, "entry", "Duplicate", "Retire", 300_000)
    };
    assert_eq!(run(&source).unwrap(), Value::Bool(true));
    let error = run(&source.replace("impl Duplicate for Item {}", "")).unwrap_err();
    assert!(
        error.contains("Copy component has no Copy capability"),
        "{error}"
    );
    let error = run(&source.replace("impl Duplicate for Tuple<Item,bool> {}", "")).unwrap_err();
    assert!(
        error.contains("computed pattern value needs independent resource"),
        "{error}"
    );
}
