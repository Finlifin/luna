//! Contextual-code methods: selection, receiver frames, clauses and contracts.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use support::run_iir as run;

#[test]
fn code_method_declarations_coexist_with_ordinary_trait_dispatch() {
    let source = "trait Builder {
            comptime fn build(code:<x:bool|-bool>)-><x:bool|-bool>;
            fn read(self)->bool;
        }
        struct Flag {}
        impl Builder for Flag {
            comptime fn build(code:<renamed:bool|-bool>)-><renamed:bool|-bool> {code}
            fn read(self)->bool {true}
        }
        comptime fn constant()->bool {true}
        fn run()->bool {const flag=constant();flag and Flag{}.as(Builder).read()}";
    let package = support::compile(source);
    let contract = package.traits.iter().find(|t| t.name == "Builder").unwrap();
    assert_eq!(contract.methods.len(), 1);
    assert_eq!(contract.methods[0].name, "read");
    assert!(package.implementations.iter().all(|implementation| {
        implementation
            .methods
            .iter()
            .all(|(name, _)| name != "build")
    }));
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_method_default_bodies_and_generic_interfaces_are_checked() {
    let source = "trait Builder where T {
            comptime fn build(code:<x:T|-T>)-><x:T|-T> {code}
            fn read(self)->bool;
        }
        struct Flag {}
        impl Builder<bool> for Flag {fn read(self)->bool {true}}
        fn run()->bool {Flag{}.as(Builder<bool>).read()}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let value_input = "trait Builder where T {comptime fn build(value:T)-><|-T>;} struct Flag {} impl Builder<bool> for Flag {comptime fn build(value:bool)-><|-bool> {'(value)}} fn run()->bool {true}";
    assert_eq!(run(value_input, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("{code}", "{'(false)}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn code_method_contracts_reject_category_context_and_missing_members() {
    let prefix =
        "trait Builder {comptime fn build(code:<x:bool|-bool>)-><x:bool|-bool>;} struct Flag {}";
    for member in [
        "",
        "comptime fn build(code:bool)-><x:bool|-bool> {'(x)}",
        "comptime fn build(code:<x:bool|-bool>)-><|-bool> {'(true)}",
        "comptime fn build(code:<x:.char'builtin|-.char'builtin>)-><x:.char'builtin|-.char'builtin> {code}",
        "fn build(code:bool)->bool {code}",
    ] {
        let source = format!("{prefix} impl Builder for Flag {{{member}}} fn run()->bool {{true}}");
        let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            failure.contains(if member.is_empty() {
                "missing required trait method"
            } else {
                "does not match its trait signature"
            }),
            "{source}: {failure}"
        );
    }
}

#[test]
fn code_method_declarations_preserve_comptime_sibling_dispatch() {
    let source = "trait Builder {
            comptime fn build(code:<|-T>)-><|-T> where T;
            comptime fn ready(self)->bool;
        }
        struct Flag {}
        impl Builder for Flag {
            comptime fn build(code:<|-U>)-><|-U> where U {code}
            comptime fn ready(self)->bool {true}
        }
        fn run()->bool {const ready=Flag{}.as(Builder).ready();ready}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_method_declarations_check_defaults_contracts_and_self_domains() {
    for declaration in [
        "comptime fn build(value:Self)-><|-bool> {'(true)}",
        "comptime fn build(.code:<x:bool|-bool> = '(x))-><x:bool|-bool> {code}",
        "comptime fn build(flag:bool)-><|-bool> where requires flag;",
    ] {
        let source = format!("trait Builder {{{declaration}}} fn run()->bool {{true}}");
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
    }
    for source in [
        "fn runtime()->bool {true} trait Builder {comptime fn build(.flag:bool=runtime())-><|-bool>;} fn run()->bool {true}",
        "trait Builder {comptime fn build(code:<x:.Type'builtin|-bool>)-><|-bool>;} fn run()->bool {true}",
        "trait Builder {fn build(code:<|-bool>)-><|-bool>;} fn run()->bool {true}",
        "comptime fn build(code:<|-bool>)-><|-bool>; fn run()->bool {true}",
        "trait Builder {comptime fn build(code:<x:bool|-bool>)-><|-bool> where requires x;} fn run()->bool {true}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn trait_code_method_dispatch_requires_an_implementation() {
    for source in [
        "trait Builder {comptime fn build()-><|-bool> {'(true)}} struct Flag {} fn run()->bool {~<Flag.as(Builder).build()>}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn static_code_methods_emit_closed_and_contextual_code() {
    let source = "struct Builder {
            pub comptime fn closed()-><|-bool> {'(true)}
            pub comptime fn forward(code:<input:bool|-bool>)-><result:bool|-bool> {code}
        }
        fn run(value:bool)->bool {
            const original:<x:bool|-bool> = '(x);
            const code=Builder.forward(original);
            ~<Builder.closed()> and ~[result=value]<code>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
}

#[test]
fn static_code_methods_infer_and_accept_method_type_arguments() {
    let source = "struct Builder {} impl Builder {
            comptime fn identity(code:<|-T>)-><|-T> where T {code}
        }
        fn run()->.u8'builtin {~<Builder.identity<.u8'builtin>('(200))>}
        fn inferred()->.u8'builtin {const code:<|-.u8'builtin> = Builder.identity('(201));~<code>}
        fn unannotated()->bool {const code=Builder.identity('(true));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
    assert_eq!(run(source, "inferred", vec![]), Value::UnsignedInteger(201));
    assert_eq!(run(source, "unannotated", vec![]), Value::Bool(true));
}

#[test]
fn static_code_methods_reuse_default_and_argument_evaluation_order() {
    let source="typealias Count=.u8'builtin;
        comptime fn mark(count:*Count)->bool where requires count.* == 0 {count.*+=1;true}
        struct Builder {
            pub comptime fn build(first:bool,.second:bool=true)-><|-bool> where requires first {'(first and second)}
        }
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=Builder.build(mark(count.ref));
            if count == 1 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Builder.build(mark(count.ref))", "Builder.build(false)");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn static_code_methods_reuse_spread_and_property_routing() {
    let source = "struct Builder {
            pub comptime fn flags(...items:(bool,bool))-><|-bool> {let (a,b)=items;'(a and b)}
            pub comptime fn pack(...properties:T)-><|-T> where T {'(properties)}
        }
        fn run()->bool {
            const flags=Builder.flags(...(true,true));
            const packed=Builder.pack {flag:true};
            let record=~<packed>;
            ~<flags> and record.flag
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn static_code_methods_enforce_method_type_requirements() {
    let source = "trait Allowed {} impl Allowed for bool {}
        struct Builder {} impl Builder {
            comptime fn build(code:<|-T>)-><|-T> where T :- Allowed {code}
        }
        fn run()->bool {const code=Builder.build<bool>('(true));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("build<bool>('(true))", "build<.char'builtin>('( 'x'))");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("trait") || failure.contains("requirement"),
        "{failure}"
    );
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn static_code_methods_reject_runtime_data_and_callable_escape() {
    for body in [
        "const code=Builder.build(value);~<code>",
        "let method=Builder.build;true",
    ] {
        let source = format!(
            "struct Builder {{pub comptime fn build(value:bool)-><|-bool> {{'(value)}}}}
            fn run(value:bool)->bool {{{body}}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn static_code_method_classification_preserves_ordinary_same_named_members() {
    let source = "struct Code {} impl Code {comptime fn build()-><|-bool> {'(true)}}
        struct Data {} impl Data {comptime fn build()->bool {false}}
        fn run()->bool {const code=Code.build();const flag=Data.build();~<code> and not flag}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn static_code_methods_preserve_nested_member_visibility() {
    let source = "mod hidden {pub struct Builder {private comptime fn build()-><|-bool> {'(true)}}}
        fn run()->bool {~<hidden.Builder.build()>}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
    assert_eq!(
        run(
            &source.replace("private comptime fn", "pub comptime fn"),
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn generic_container_code_methods_substitute_code_contexts() {
    let source = "struct Builder where T {
        pub comptime fn forward(code:<input:T|-T>)-><result:T|-T> {code}
    }
    fn run(value:bool)->bool {
        const original:<x:bool|-bool> = '(x);
        const code = Builder<bool>.forward(original);
        ~[result=value]<code>
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
}

#[test]
fn generic_impl_code_methods_substitute_body_and_data_defaults() {
    let source = "struct Builder where T {}
    impl Builder<T> where T {
        comptime fn build(value:T,.code:<|-T> = '(value))-><|-T> {let copied:T=value;code}
    }
    fn run()->bool {~<Builder<bool>.build(true)>}
    fn byte()->.u8'builtin {~<Builder<.u8'builtin>.build(201)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "byte", vec![]), Value::UnsignedInteger(201));
}

#[test]
fn generic_code_methods_keep_parent_and_method_types_distinct() {
    let source = "struct Builder where T {} impl Builder<T> where T {
        comptime fn build(left:<|-T>,right:<|-U>)-><|-U> where U {right}
    }
    fn run()->bool {~<Builder<.u8'builtin>.build<bool>('(200),'(true))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_methods_restore_recursive_instantiation_environments() {
    let source = "struct Builder where T {} impl Builder<T> where T {
        comptime fn build(value:T,again:bool)-><|-T> {
            if again {let ignored:<|-bool> = Builder<bool>.build(false,false);}
            let restored:T=value;
            '(restored)
        }
    }
    fn run()->.u8'builtin {~<Builder<.u8'builtin>.build(201,true)>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(201));
}

#[test]
fn generic_code_methods_check_captured_type_requirements() {
    let source = "struct Builder where T {} impl Builder<T> where T {
        comptime fn build(code:<|-U>)-><|-U> where U, requires U == T {code}
    }
    fn run()->bool {~<Builder<bool>.build<bool>('(true))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Builder<bool>", "Builder<.u8'builtin>");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("requirement") || failure.contains("precondition"),
        "{failure}"
    );
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_methods_default_type_parameters_from_parent() {
    let source = "struct Builder where T {} impl Builder<T> where T {
        comptime fn build(code:<|-U>)-><|-U> where .U:.Type'builtin=T {code}
    }
    fn run()->bool {const code=Builder<bool>.build('(true));~<code>}
    fn byte()->.u8'builtin {const code=Builder<.u8'builtin>.build('(203));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "byte", vec![]), Value::UnsignedInteger(203));
}

#[test]
fn generic_code_methods_preserve_self_and_nominal_results() {
    let source = "struct Builder where T {value:T}
        impl Builder<T> where T {
            comptime fn build(value:Self)-><|-Self> {'(value)}
        }
        fn run()->bool {
            const code=Builder<bool>.build(Builder<bool>{value:true});
            let value=~<code>;value.value
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_methods_do_not_erase_parent_formation_requirements() {
    let source = "trait Allowed {} impl Allowed for bool {}
        struct Builder where T :- Allowed {
            pub comptime fn build(code:<|-T>)-><|-T> {code}
        }
        fn run()->bool {~<Builder<bool>.build('(true))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "Builder<bool>.build('(true))",
        "Builder<.u8'builtin>.build('(200))",
    );
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("trait") || failure.contains("requirement"),
        "{failure}"
    );
}

#[test]
fn trait_code_methods_invoke_default_bodies() {
    let source = "trait Builder {comptime fn build()-><|-bool> {'(true)}}
        struct Flag {} impl Builder for Flag {}
        fn run()->bool {const code=Flag.as(Builder).build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_code_methods_remap_implementation_contexts() {
    let source = "trait Builder {comptime fn build(code:<x:bool|-bool>)-><y:bool|-bool>;}
        struct Flag {} impl Builder for Flag {
            comptime fn build(input:<a:bool|-bool>)-><b:bool|-bool> {input}
        }
        fn run(value:bool)->bool {
            const original:<source:bool|-bool> = '(source);
            const code=Flag.as(Builder).build(original);
            ~[y=value]<code>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
}

#[test]
fn trait_code_methods_use_public_parameter_names_and_defaults() {
    let source = "trait Builder {comptime fn build(.flag:bool=true)-><|-bool>;}
        struct Flag {} impl Builder for Flag {
            comptime fn build(.renamed:bool=false)-><|-bool> {'(renamed)}
        }
        fn run()->bool {~<Flag.as(Builder).build()>}
        fn named()->bool {~<Flag.as(Builder).build(.flag=false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "named", vec![]), Value::Bool(false));
}

#[test]
fn trait_code_methods_check_public_preconditions() {
    let source = "trait Builder {comptime fn build(flag:bool)-><|-bool> where requires flag;}
        struct Flag {} impl Builder for Flag {comptime fn build(input:bool)-><|-bool> {'(true)}}
        fn run()->bool {~<Flag.as(Builder).build(true)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("build(true)", "build(false)");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn trait_code_methods_specialize_trait_and_method_types() {
    let source="trait Builder where T {comptime fn build(value:T,code:<|-U>)-><|-U> where U;}
        struct Flag {} impl Builder<bool> for Flag {
            comptime fn build(input:bool,result:<|-V>)-><|-V> where V {result}
        }
        fn run()->.u8'builtin {const code=Flag.as(Builder<bool>).build<.u8'builtin>(true,'(201));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(201));
}

#[test]
fn trait_code_method_defaults_forward_through_abstract_self() {
    let source = "trait Builder {
        comptime fn build(code:<|-T>)-><|-T> where T;
        comptime fn wrap(code:<|-T>)-><|-T> where T {Self.as(Builder).build<T>(code)}
    }
    struct Flag {} impl Builder for Flag {comptime fn build(code:<|-U>)-><|-U> where U {code}}
    fn run()->bool {const code=Flag.as(Builder).wrap('(true));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_code_methods_preserve_single_input_evaluation() {
    let source = "typealias Count=.Integer'builtin;
        comptime fn next(count:*Count)->bool {count.*+=1;true}
        trait Builder {comptime fn build(flag:bool)-><|-bool>;}
        struct Flag {} impl Builder for Flag {comptime fn build(input:bool)-><|-bool> {'(input)}}
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=Flag.as(Builder).build(next(count.ref));
            if count == 1 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_code_methods_preserve_inherited_view_requirements() {
    let source = "trait Parent {comptime fn build()-><|-bool> {'(true)}}
        trait Child :- Parent {}
        struct Flag {} impl Parent for Flag {} impl Child for Flag {}
        fn run()->bool {const code=Flag.as(Child).build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("impl Child for Flag {}", "");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("interface") || failure.contains("trait"),
        "{failure}"
    );
}

#[test]
fn trait_code_methods_keep_implicit_context_connections() {
    let source = "trait Builder {comptime fn build(code:<x:bool|-bool>)-><x:bool|-bool>;}
        struct Flag {} impl Builder for Flag {
            comptime fn build(input:<a:bool|-bool>)-><a:bool|-bool> {input}
        }
        fn run(value:bool)->bool {
            const code=Flag.as(Builder).build('[source=value](value));
            ~<code>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
}

#[test]
fn trait_code_methods_route_spread_and_property_inputs() {
    let source = "trait Builder {
        comptime fn flags(...items:(bool,bool))-><|-bool>;
        comptime fn pack(...properties:T)-><|-T> where T;
    }
    struct Flag {} impl Builder for Flag {
        comptime fn flags(...values:(bool,bool))-><|-bool> {let (a,b)=values;'(a and b)}
        comptime fn pack(...fields:U)-><|-U> where U {'(fields)}
    }
    fn run()->bool {
        const flags=Flag.as(Builder).flags(...(true,true));
        const packed=Flag.as(Builder).pack {flag:true};
        let record=~<packed>;~<flags> and record.flag
    }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_code_methods_specialize_generic_implementation_captures() {
    let source = "trait Builder where T {comptime fn build(value:T)-><|-T>;}
        struct Flag where U {} impl Builder<U> for Flag<U> where U {
            comptime fn build(input:U)-><|-U> {let copied:U=input;'(copied)}
        }
        fn run()->bool {const code=Flag<bool>.as(Builder<bool>).build(true);~<code>}
        fn byte()->.u8'builtin {~<Flag<.u8'builtin>.as(Builder<.u8'builtin>).build(202)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "byte", vec![]), Value::UnsignedInteger(202));
}

#[test]
fn trait_code_methods_check_public_postconditions() {
    let source = "trait Builder {comptime fn build(flag:bool)-><|-bool> where ensures flag;}
        struct Flag {} impl Builder for Flag {comptime fn build(input:bool)-><|-bool> {'(true)}}
        fn run()->bool {~<Flag.as(Builder).build(true)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("build(true)", "build(false)");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("postcondition"), "{failure}");
}

#[test]
fn trait_code_methods_defer_selection_in_generic_generator_definitions() {
    let source = "trait Builder {comptime fn build()-><|-bool>;}
        struct Flag {} impl Builder for Flag {comptime fn build()-><|-bool> {'(true)}}
        comptime fn generate()-><|-bool> where T :- Builder {T.as(Builder).build()}
        fn run()->bool {~<generate<Flag>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_method_value_receivers_generate_from_nominal_data() {
    let source = "struct Builder {flag:bool} impl Builder {
        comptime fn build(self)-><|-bool> {let flag=self.flag;'(flag)}
    }
    fn run()->bool {const code=Builder{flag:true}.build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(
        run(&source.replace("flag:true", "flag:false"), "run", vec![]),
        Value::Bool(false)
    );
}

#[test]
fn trait_code_method_value_receivers_use_selected_body() {
    let source = "trait Build {comptime fn build(self)-><|-bool>;}
        struct Builder {flag:bool} impl Build for Builder {
            comptime fn build(self)-><|-bool> {let flag=self.flag;'(flag)}
        }
        fn run()->bool {const code=Builder{flag:true}.as(Build).build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_method_value_receivers_preserve_parent_types() {
    let source = "struct Builder where T {value:T} impl Builder<T> where T {
        comptime fn build(self)-><|-T> {let value:T=self.value;'(value)}
    }
    fn run()->.u8'builtin {const code=Builder<.u8'builtin>{value:200}.build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn code_method_value_receivers_evaluate_once_before_arguments() {
    let source="typealias Count=.Integer'builtin;
        struct Builder {flag:bool}
        comptime fn make(count:*Count)->Builder {count.* = count.* * 10 + 1;Builder{flag:true}}
        comptime fn next(count:*Count)->bool {let ordered=count.* == 1;count.* = count.* * 10 + 2;ordered}
        impl Builder {comptime fn build(self,flag:bool)-><|-bool> {let result=self.flag and flag;'(result)}}
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=make(count.ref).build(next(count.ref));
            if count == 12 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_code_method_value_receivers_forward_from_defaults() {
    let source = "trait Build {
        comptime fn build(self)-><|-bool>;
        comptime fn wrap(self)-><|-bool> {self.as(Build).build()}
    }
    struct Builder {flag:bool} impl Build for Builder {
        comptime fn build(self)-><|-bool> {let flag=self.flag;'(flag)}
    }
    fn run()->bool {const code=Builder{flag:true}.as(Build).wrap();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_method_value_receivers_keep_code_contexts() {
    let source = "struct Builder {flag:bool} impl Builder {
        comptime fn build(self,code:<x:bool|-bool>)-><x:bool|-bool> {
            if self.flag {code} else {'(not x)}
        }
    }
    fn run(value:bool)->bool {
        const code=Builder{flag:true}.build('[input=value](value));~<code>
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
}

#[test]
fn code_method_value_receivers_route_property_and_spread_inputs() {
    let source="struct Builder {flag:bool} impl Builder {
        comptime fn pack(self,...properties:T)-><|-T> where T {'(properties)}
        comptime fn flags(self,...items:(bool,bool))-><|-bool> {let(a,b)=items;let flag=self.flag;'(flag and a and b)}
    }
    fn run()->bool {
        const code=Builder{flag:true}.pack {flag:true};let record=~<code>;
        const flags=Builder{flag:true}.flags(...(true,true));record.flag and ~<flags>
    }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_method_value_receivers_reject_runtime_inputs_and_missing_receivers() {
    let prefix = "struct Builder {flag:bool} impl Builder {
        comptime fn build(self)-><|-bool> {let flag=self.flag;'(flag)}
        comptime fn static_build()-><|-bool> {'(true)}
    }";
    for body in [
        "const code=value.build();~<code>",
        "~<Builder.build()>",
        "~<Builder{flag:true}.static_build()>",
    ] {
        let source = format!("{prefix} fn run(value:Builder)->bool {{{body}}}");
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn code_method_value_receivers_enforce_receiver_contracts() {
    let source = "struct Builder {flag:bool} impl Builder {
        comptime fn build(self)-><|-bool> where requires self.flag {'(true)}
    }
    fn run()->bool {~<Builder{flag:true}.build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("flag:true", "flag:false");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn code_method_value_receivers_support_primitive_literals() {
    let source = "trait Invert {comptime fn inverted(self)-><|-bool>;}
    impl Invert for bool {
        comptime fn inverted(self)-><|-bool> {let flag=not self;'(flag)}
    }
    fn run()->bool {const code=(true).as(Invert).inverted();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(false));
}

#[test]
fn code_method_value_clauses_specialize_inherent_bodies() {
    let source = "struct Builder {} impl Builder {
        comptime fn build()-><|-bool> where flag:bool {'(flag)}
    }
    fn run()->bool {const code=Builder.build<true>();~<code>}
    fn disabled()->bool {~<Builder.build<false>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "disabled", vec![]), Value::Bool(false));
}

#[test]
fn code_method_value_clauses_preserve_trait_binder_sorts() {
    let source = "trait Build {comptime fn build()-><|-bool> where flag:bool;}
        struct Builder {} impl Build for Builder {
            comptime fn build()-><|-bool> where enabled:bool {'(enabled)}
        }
        fn run()->bool {const code=Builder.as(Build).build<true>();~<code>}
        fn disabled()->bool {~<Builder.as(Build).build<false>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "disabled", vec![]), Value::Bool(false));
}

#[test]
fn code_method_value_clauses_use_public_defaults_and_names() {
    let source = "trait Build {comptime fn build()-><|-bool> where .flag:bool=true;}
        struct Builder {} impl Build for Builder {
            comptime fn build()-><|-bool> where .enabled:bool=false {'(enabled)}
        }
        fn run()->bool {~<Builder.as(Build).build()>}
        fn disabled()->bool {~<Builder.as(Build).build<.flag=false>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "disabled", vec![]), Value::Bool(false));
}

#[test]
fn code_method_value_clauses_support_dependent_domains() {
    let source = "trait Build {comptime fn build()-><|-T> where T,value:T;}
        struct Builder {} impl Build for Builder {
            comptime fn build()-><|-U> where U,item:U {'(item)}
        }
        fn run()->.u8'builtin {~<Builder.as(Build).build<.u8'builtin,201>()>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(201));
}

#[test]
fn code_method_value_clauses_reject_unused_domain_mismatches() {
    let prefix = "trait Build {comptime fn build()-><|-bool> where flag:bool;} struct Builder {}";
    for domain in [".u8'builtin", ".Type'builtin"] {
        let source = format!(
            "{prefix} impl Build for Builder {{comptime fn build()-><|-bool> where flag:{domain} {{'(true)}}}} fn run()->bool {{true}}"
        );
        let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(failure.contains("trait signature"), "{failure}");
    }
}

#[test]
fn code_method_value_clauses_keep_program_contracts_out_of_type_proofs() {
    let source = "trait Build {comptime fn build()-><|-bool> where enabled:bool, requires enabled;}
        struct Builder {} impl Build for Builder {
            comptime fn build()-><|-bool> where flag:bool {'(flag)}
        }
        fn run()->bool {~<Builder.as(Build).build<true>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("build<true>", "build<false>");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn code_method_value_clauses_work_with_receivers_and_default_bodies() {
    let source = "trait Build {
        comptime fn build(self)-><|-bool> where enabled:bool {'(enabled)}
        comptime fn wrap(self)-><|-bool> where flag:bool {self.as(Build).build<flag>()}
    }
    struct Builder {} impl Build for Builder {}
    fn run()->bool {const code=Builder{}.as(Build).wrap<true>();~<code>}
    fn disabled()->bool {~<Builder{}.as(Build).wrap<false>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "disabled", vec![]), Value::Bool(false));
}

#[test]
fn code_method_value_clauses_preserve_indexed_type_propositions() {
    let source="pure comptime fn Tagged<flag:bool>->.Type'builtin {bool}
        struct Builder {} impl Builder {
            comptime fn build()-><|-bool> where flag:bool, requires Tagged<flag> == Tagged<true> {'(flag)}
        }
        fn run()->bool {~<Builder.build<true>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("build<true>", "build<false>");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("requirement") || failure.contains("precondition"),
        "{failure}"
    );
}

#[test]
fn code_method_parent_value_clauses_reach_trait_default_bodies() {
    let source = "trait Build where enabled:bool {comptime fn build()-><|-bool> {'(enabled)}}
        struct Builder {} impl Build<true> for Builder {} impl Build<false> for Builder {}
        fn run()->bool {const code=Builder.as(Build<true>).build();~<code>}
        fn disabled()->bool {~<Builder.as(Build<false>).build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "disabled", vec![]), Value::Bool(false));
}

#[test]
fn code_method_parent_value_clauses_supply_method_defaults() {
    let source = "trait Build where enabled:bool {
        comptime fn build()-><|-bool> where .flag:bool=enabled {'(flag)}
    }
    struct Builder {} impl Build<true> for Builder {} impl Build<false> for Builder {}
    fn run()->bool {~<Builder.as(Build<true>).build()>}
    fn disabled()->bool {~<Builder.as(Build<false>).build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "disabled", vec![]), Value::Bool(false));
}

#[test]
fn code_method_parent_value_clauses_instantiate_dependent_domains() {
    let source="trait Build where T,value:T {comptime fn build()-><|-T> {'(value)}}
        struct Builder {} impl Build<bool,true> for Builder {} impl Build<.u8'builtin,201> for Builder {}
        fn run()->bool {~<Builder.as(Build<bool,true>).build()>}
        fn byte()->.u8'builtin {~<Builder.as(Build<.u8'builtin,201>).build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "byte", vec![]), Value::UnsignedInteger(201));
}

#[test]
fn code_method_parent_value_clauses_keep_program_preconditions() {
    let source = "trait Build where enabled:bool {
        comptime fn build()-><|-bool> where requires enabled {'(true)}
    }
    struct Builder {} impl Build<true> for Builder {} impl Build<false> for Builder {}
    fn run()->bool {~<Builder.as(Build<true>).build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("as(Build<true>)", "as(Build<false>)");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn code_method_parent_value_clauses_restore_nested_instances() {
    let source = "trait Build where enabled:bool {
        comptime fn build()-><|-bool> {
            if enabled {let nested:<|-bool> = Builder.as(Build<false>).build();}
            '(enabled)
        }
    }
    struct Builder {} impl Build<true> for Builder {} impl Build<false> for Builder {}
    fn run()->bool {~<Builder.as(Build<true>).build()>}
    fn disabled()->bool {~<Builder.as(Build<false>).build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(run(source, "disabled", vec![]), Value::Bool(false));
}

#[test]
fn code_method_parent_value_clauses_survive_trait_inheritance() {
    let source = "trait Parent where enabled:bool {comptime fn build()-><|-bool> {'(enabled)}}
        trait Child :- Parent<true> {}
        struct Builder {} impl Parent<true> for Builder {} impl Child for Builder {}
        fn run()->bool {const code=Builder.as(Child).build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("impl Child for Builder {}", "");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn code_method_parent_value_clauses_check_unused_defaults() {
    let source = "trait Build where enabled:bool {
        comptime fn build()-><|-bool> where .flag:bool='x' {'(flag)}
    }
    struct Builder {} impl Build<true> for Builder {}
    fn run()->bool {~<Builder.as(Build<true>).build<.flag=true>()>}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn code_method_parent_value_clauses_form_indexed_code_signatures() {
    let source = "pure comptime fn Tagged<flag:bool>->.Type'builtin {bool}
        trait Build where flag:bool {
            comptime fn keep(code:<|-Tagged<flag>>)-><|-Tagged<flag>> {code}
        }
        struct Builder {} impl Build<true> for Builder {}
        fn run()->bool {true}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid=source.replace("impl Build<true> for Builder {}","impl Build<true> for Builder {comptime fn keep(code:<|-Tagged<false>>)-><|-Tagged<false>> {code}}");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("trait signature"), "{failure}");
}

#[test]
fn code_method_program_refinement_rejects_unused_strengthening() {
    let source = "trait Build {comptime fn build(flag:bool)-><|-bool>;}
        struct Builder {} impl Build for Builder {
            comptime fn build(input:bool)-><|-bool> where requires input {'(true)}
        }
        fn run()->bool {true}";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        failure.contains("strengthens trait program preconditions"),
        "{failure}"
    );
}

#[test]
fn code_method_program_refinement_remaps_data_and_preserves_code_contexts() {
    let source = "trait Build {
            comptime fn build(flag:bool,code:<x:bool|-bool>)-><x:bool|-bool> where requires flag;
        }
        struct Builder {} impl Build for Builder {
            comptime fn build(renamed:bool,value:<y:bool|-bool>)-><y:bool|-bool> where requires renamed {value}
        }
        fn run()->bool {let x=true;const code=Builder.as(Build).build(true,'[x=x](x));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_method_program_refinement_accepts_weaker_arithmetic_domain() {
    let source = "trait Build {
            comptime fn build(count:.u8'builtin)-><|-bool> where requires count > 10;
        }
        struct Builder {} impl Build for Builder {
            comptime fn build(n:.u8'builtin)-><|-bool> where requires n > 0 {'(true)}
        }
        fn run()->bool {~<Builder.as(Build).build(11)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("requires n > 0", "requires n > 20");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("strengthens trait program preconditions"),
        "{failure}"
    );
}

#[test]
fn code_method_program_refinement_remaps_method_value_clauses() {
    let source = "trait Build {
            comptime fn build()-><|-bool> where flag:bool, requires flag;
        }
        struct Builder {} impl Build for Builder {
            comptime fn build()-><|-bool> where renamed:bool, requires renamed {'(true)}
        }
        fn run()->bool {~<Builder.as(Build).build<true>()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("requires renamed", "requires not renamed");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("strengthens trait program preconditions"),
        "{failure}"
    );
}

#[test]
fn code_method_program_refinement_substitutes_parent_value_clauses() {
    let source = "trait Build where enabled:bool {
            comptime fn build(flag:bool)-><|-bool> where requires enabled and flag;
        }
        struct Builder {} impl Build<true> for Builder {
            comptime fn build(renamed:bool)-><|-bool> where requires renamed {'(true)}
        }
        fn run()->bool {~<Builder.as(Build<true>).build(true)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("enabled and flag", "enabled or flag");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("strengthens trait program preconditions"),
        "{failure}"
    );
}

#[test]
fn code_method_program_refinement_substitutes_parent_integer_clauses() {
    let source = "trait Build where limit:.u8'builtin {
            comptime fn build(count:.u8'builtin)-><|-bool> where requires count > limit;
        }
        struct Builder {} impl Build<10> for Builder {
            comptime fn build(n:.u8'builtin)-><|-bool> where requires n > 0 {'(true)}
        }
        fn run()->bool {~<Builder.as(Build<10>).build(11)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("requires n > 0", "requires n > 20");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("strengthens trait program preconditions"),
        "{failure}"
    );
}

#[test]
fn code_method_program_refinement_checks_predicate_definedness() {
    let source = "trait Build {
            comptime fn build(count:.u8'builtin)-><|-bool> where requires count > 0;
        }
        struct Builder {} impl Build for Builder {
            comptime fn build(n:.u8'builtin)-><|-bool> where requires 1 / n <= 1 {'(true)}
        }
        fn run()->bool {~<Builder.as(Build).build(3)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("where requires count > 0", "");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("definedness is not proven"), "{failure}");
}

#[test]
fn code_method_program_refinement_accepts_unbounded_integer_half_lines() {
    let source = "trait Build {
            comptime fn build(count:.Integer'builtin)-><|-bool> where requires count > 10;
        }
        struct Builder {} impl Build for Builder {
            comptime fn build(n:.Integer'builtin)-><|-bool> where requires n > 0 {'(true)}
        }
        fn run()->bool {~<Builder.as(Build).build(11)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("requires n > 0", "requires n > 20");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("definedness is not proven"), "{failure}");
}

#[test]
fn reference_code_method_receivers_write_back_to_generator_owned_state() {
    let source = "struct Builder {flag:bool}
        impl Builder {
            comptime fn build(*self)-><|-bool> {self.*.flag=true;let flag=self.*.flag;'(flag)}
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder {flag:false};
            let code=builder.build();
            if builder.flag {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_reference_code_methods_share_state_and_keep_earlier_quotes() {
    let source = "trait Build {comptime fn build(*self)-><|-bool>;}
        struct Builder {flag:bool}
        impl Build for Builder {
            comptime fn build(*self)-><|-bool> {self.*.flag=not self.*.flag;let flag=self.*.flag;'(flag)}
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder {flag:false};
            let first=builder.as(Build).build();
            let second=builder.as(Build).build();
            if not builder.flag {first} else {second}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn pointer_generator_inputs_preserve_writeback_and_check_contracts() {
    let source = "struct Builder {flag:bool}
        comptime fn build(state:*Builder)-><|-bool> where requires not state.*.flag {
            state.*.flag=true;let flag=state.*.flag;'(flag)
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder {flag:false};
            let code=build(builder.ref);
            if builder.flag {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Builder {flag:false}", "Builder {flag:true}");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn reference_code_methods_cannot_embed_comptime_pointers_in_runtime_code() {
    let source = "struct Builder {flag:bool}
        impl Builder {comptime fn leak(*self)-><|-*Builder> {'(self)}}
        comptime fn outer()-><|-*Builder> {
            var builder=Builder {flag:false};builder.leak()
        }
        fn run()->*Builder {~<outer()>}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn pointer_generator_arguments_evaluate_once_in_source_order() {
    let source = "struct Builder {flag:bool}
        comptime fn build(state:*Builder,flag:bool)-><|-bool> {
            state.*.flag=flag;let saved=state.*.flag;'(saved)
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder {flag:false};var count=0;
            let code=build(do {count+=1;builder.ref}, do {count+=1;count == 2});
            if builder.flag and count == 2 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_reference_code_methods_preserve_parent_types_and_writeback() {
    let source = "struct Builder where T {value:T}
        impl Builder<T> where T {
            comptime fn replace(*self,value:T)-><|-T> {self.*.value=value;let saved=self.*.value;'(saved)}
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder<bool> {value:false};
            let code=builder.replace(true);
            if builder.value {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_default_reference_code_methods_forward_the_same_owner() {
    let source = "trait Build {
            comptime fn build(*self)-><|-bool>;
            comptime fn again(*self)-><|-bool> {self.*.as(Build).build()}
        }
        struct Builder {flag:bool}
        impl Build for Builder {
            comptime fn build(*self)-><|-bool> {self.*.flag=true;let saved=self.*.flag;'(saved)}
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder {flag:false};
            let code=builder.as(Build).again();
            if builder.flag {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn reference_code_dispatch_checks_public_pointer_contracts_after_writeback() {
    let source = "trait Build {
            comptime fn build(*self,state:*bool)-><|-bool> where requires not state.*, ensures state.*;
        }
        struct Builder {}
        impl Build for Builder {
            comptime fn build(*self,output:*bool)-><|-bool> {output.* = true;'(true)}
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder {};var flag=false;
            let code=builder.as(Build).build(flag.ref);
            if flag {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for (from, to, diagnostic) in [
        ("var flag=false", "var flag=true", "precondition"),
        ("output.* = true", "output.* = false", "postcondition"),
    ] {
        let rejected = source.replace(from, to);
        let error = support::compile_with_protocols(&rejected, &Default::default()).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn pointer_generator_aliases_observe_each_write_across_statements() {
    for (right, expected, final_left) in [("left", "true", "false"), ("right", "false", "true")] {
        let source = format!(
            "comptime fn build(first:*bool,second:*bool,expected:bool)-><|-bool> {{
                first.* = true;
                let seen=second.*;
                second.* = false;
                '(seen == expected)
            }}
            comptime fn outer()-><|-bool> {{
                var left=false;var right=false;
                let code=build(left.ref,{right}.ref,{expected});
                if left == {final_left} and not right {{code}} else {{'(false)}}
            }}
            fn run()->bool {{~<outer()>}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generator_local_pointer_bindings_keep_aliases_and_refresh_indirect_owners() {
    let source = "comptime fn build()-><|-bool> {
            var first=false;var second=false;
            var pointer=first.ref;
            let alias=pointer;
            pointer.* = true;
            let seen=first and alias.*;
            pointer=second.ref;
            pointer.* = true;
            alias.* = false;
            let result=seen and not first and second;
            '(result)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_local_pointers_reject_dead_owners_and_runtime_exports() {
    for source in [
        "comptime fn build()-><|-bool> {let pointer=do {var inner=false;inner.ref};let value=pointer.*;'(value)} fn run()->bool {~<build()>}",
        "comptime fn build()-><|-*bool> {var flag=false;let pointer=flag.ref;'(pointer)} fn run()->*bool {~<build()>}",
        "comptime fn build()-><|-bool> {var first=false;var second=false;let pointer=first.ref;pointer=second.ref;'(true)} fn run()->bool {~<build()>}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_pointer_patterns_keep_live_aliases_and_frozen_observations() {
    let source = "comptime fn build()-><|-bool> {
            var flag=false;
            let pointer and pointer.* is original=flag.ref;
            pointer.* = true;
            let result=not original and flag;
            '(result)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_pointer_tuples_destructure_without_copying_owners() {
    let source = "comptime fn build()-><|-bool> {
            var left=false;var right=false;
            let pair=(left.ref,right.ref);
            let (first,second)=pair;
            first.* = true;second.* = true;
            let result=left and right;
            '(result)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_pointer_records_transfer_as_live_data_inputs() {
    let source = "struct Links {first:*bool,second:*bool}
        comptime fn write(links:Links)-><|-bool> {
            links.first.* = true;links.second.* = true;
            let result=links.first.* and links.second.*;
            '(result)
        }
        comptime fn outer()-><|-bool> {
            var left=false;var right=false;
            let links=Links {first:left.ref,second:right.ref};
            let code=write(links);
            if left and right {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn pointer_aggregates_do_not_bypass_callback_capture_checks() {
    let source = "typealias Reader=%{flag:bool} comptime fn()->bool;
        comptime fn make()->Reader {var flag=false;|| {flag=not flag;flag}}
        comptime fn build()-><|-bool> {
            var owned=false;
            let packet=(owned.ref,make());
            let (pointer,action)=packet;
            let result=action();
            '(result)
        }
        fn run()->bool {~<build()>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("mutates captured state"), "{error}");
}

#[test]
fn generator_pointer_nominal_wrappers_preserve_payload_identity() {
    let source = "enum Link {some(*bool)}
        newtype Address=*bool;
        comptime fn build()-><|-bool> {
            var flag=false;
            let wrapped=flag.ref.as(Address);
            let link=Link.some(wrapped.as(*bool));
            let .some(pointer)=link;
            pointer.* = true;
            let result=flag;
            '(result)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn pointer_aggregates_do_not_admit_runtime_callbacks() {
    let source = "struct Packet {pointer:*bool,callback:fn()->bool}
        fn runtime()->bool {true}
        comptime fn build()-><|-bool> {
            var flag=false;
            let packet=Packet {pointer:flag.ref,callback:runtime};
            '(true)
        }
        fn run()->bool {~<build()>}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn generator_live_variadic_inputs_preserve_aliases_and_frozen_siblings() {
    let source = "comptime fn write(...items:(*bool,*bool,bool))-><|-bool> {
        let (first,second,value)=items;
        first.* = value;
        let observed=second.*;
        '(observed)
    }
    comptime fn outer()-><|-bool> {
        var flag=false;
        let code=write(flag.ref,flag.ref,true);
        if flag {code} else {'(false)}
    }
    fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_live_spread_packets_evaluate_once_before_later_arguments() {
    for signature in [
        "first:*bool,second:*bool,value:bool",
        "...items:(*bool,*bool,bool)",
    ] {
        let binding = if signature.starts_with("...") {
            "let(first,second,value)=items;"
        } else {
            ""
        };
        let source = format!(
            "comptime fn write({signature})-><|-bool> {{
            {binding}
            first.* = value;let result=second.*;'(result)
        }}
        comptime fn outer()-><|-bool> {{
            var flag=false;var visited=false;
            let code=write(...do {{visited=not visited;(flag.ref,flag.ref)}},visited);
            if flag and visited {{code}} else {{'(false)}}
        }}
        fn run()->bool {{~<outer()>}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generator_live_property_packets_preserve_writeback() {
    let source = "comptime fn schema(...properties:T)->T where T {properties}
    typealias Fields=(do {let flag=false;schema {pointer:flag.ref,value:true}})'type;
    comptime fn write(...fields:Fields)-><|-bool> {
        fields.pointer.* = fields.value;
        let result=fields.pointer.*;'(result)
    }
    comptime fn outer()-><|-bool> {
        var flag=false;
        let code=write {pointer:flag.ref,value:true};
        if flag {code} else {'(false)}
    }
    fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_live_collections_and_spreads_cannot_export_pointers() {
    for argument in ["flag.ref,true", "...do {(flag.ref,true)}"] {
        let source = format!(
            "comptime fn export(...items:(*bool,bool))-><|-(*bool,bool)> {{'(items)}}
            comptime fn outer()-><|-(*bool,bool)> {{
                var flag=false;export({argument})
            }}
            fn run()->(*bool,bool) {{~<outer()>}}"
        );
        assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
    }
}

#[test]
fn generator_postconditions_keep_entry_values_after_parameter_updates() {
    let source = "comptime fn build(flag:bool)-><|-bool>
        where ensures flag and not flag'old {
        let pointer=flag.ref;pointer.* = true;'(flag)
    }
    fn run()->bool {~<build(false)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_entry_aggregates_copy_fields_but_not_pointer_targets() {
    let source = "struct State {flag:bool,target:*bool}
        comptime fn build(state:State)-><|-bool>
            where ensures state.flag and not state'old.flag and state'old.target.* {
            let pointer=state.ref;
            pointer.*.flag = true;
            state.target.* = true;
            '(true)
        }
        comptime fn outer()-><|-bool> {
            var target=false;
            build(State {flag:false,target:target.ref})
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("not state'old.flag", "state'old.flag");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("postcondition rejected"), "{error}");
}

#[test]
fn generator_entry_values_are_isolated_across_nested_calls() {
    let source = "comptime fn build(flag:bool)-><|-bool>
        where ensures not flag'old and flag {
        let pointer=flag.ref;pointer.* = true;
        '(true)
    }
    comptime fn outer(flag:bool)-><|-bool>
        where ensures flag'old and not flag {
        let nested=build(false);
        let pointer=flag.ref;pointer.* = false;
        nested
    }
    fn run()->bool {~<outer(true)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("not flag'old and flag", "flag'old and flag");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("postcondition rejected"), "{error}");
}

#[test]
fn trait_default_forwarding_keeps_live_aggregates_and_public_entry_contracts() {
    let source = "struct State {flag:bool,target:*bool}
        trait Build {
            comptime fn build(*self,state:State)-><|-bool>
                where ensures not state'old.flag and state'old.target.*;
            comptime fn again(*self,state:State)-><|-bool>
                where ensures not state'old.flag and state'old.target.* {
                self.*.as(Build).build(state)
            }
        }
        struct Builder {}
        impl Build for Builder {
            comptime fn build(*self,input:State)-><|-bool> {
                input.target.* = true;'(true)
            }
        }
        comptime fn outer()-><|-bool> {
            var builder=Builder {};var flag=false;
            let code=builder.as(Build).again(State {flag:false,target:flag.ref});
            if flag {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("input.target.* = true", "input.target.* = false");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("postcondition rejected"), "{error}");
}

#[test]
fn generator_live_matches_select_pointer_payloads() {
    let source = "enum Link {some(*bool),none}
        comptime fn build(link:Link)-><|-bool> {
            link match {
                .some(pointer)=>{pointer.* = true;let value=pointer.*;'(value)},
                .none=>'(false)
            }
        }
        comptime fn outer()-><|-bool> {
            var flag=false;
            let code=build(Link.some(flag.ref));
            if flag {code} else {'(false)}
        }
        fn run()->bool {~<outer()> and not ~<build(Link.none)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_live_let_else_exports_cells_and_evaluates_guards_once() {
    let source = "comptime fn build(enabled:bool)-><|-bool> {
            var flag=false;var visited=false;
            let (pointer,true) if do {visited=not visited;visited} = (flag.ref,enabled)
                else {return '(not visited);};
            pointer.* = true;
            let result=flag and visited;
            '(result)
        }
        fn run()->bool {~<build(true)> and ~<build(false)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_failed_guards_preserve_writes_for_later_candidates() {
    let source = "comptime fn build()-><|-bool> {
        var changed=false;var initialized=false;
        do {initialized=not initialized;true} match {
            first if do {changed=true;false} => '(false),
            second if changed => {let result=initialized and changed and second;'(result)},
            _=>'(false)
        }
    }
    fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_failed_live_let_else_guard_keeps_outer_writeback() {
    let source = "comptime fn build()-><|-bool> {
        var flag=false;
        let pointer if do {pointer.* = true;false} = flag.ref
            else {return '(flag);};
        '(false)
    }
    fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("return '(flag);", "'(flag);");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("must diverge"), "{error}");
}

#[test]
fn generator_tuple_iteration_keeps_live_pointer_aliases() {
    let source = "comptime fn build()-><|-bool> {
        var first=false;var second=false;
        let pointers=(first.ref,second.ref,first.ref);
        inline for (index,pointer) in pointers'tuple_enumerate {
            pointer.* = not pointer.*;
        }
        let result=not first and second;'(result)
    }
    fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_live_tuple_iteration_keeps_guard_and_continue_writes() {
    let source = "comptime fn build()-><|-bool> {
        var first=false;var second=false;var third=false;
        inline for (index,pointer) if do {pointer.* = true;index != 0}
            in (first.ref,second.ref,third.ref)'tuple_enumerate {
            if index == 1 {pointer.* = false;continue;}
            break;
        }
        let result=first and not second and third;'(result)
    }
    fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_tuple_iteration_preserves_nominal_pointer_fields() {
    let source = "struct Link {pointer:*bool,value:bool}
        comptime fn build()-><|-bool> {
            var first=false;var second=true;
            let items=(Link {pointer:first.ref,value:true},Link {pointer:second.ref,value:false});
            inline for (index,item) in items'tuple_enumerate {
                item.pointer.* = item.value;
            }
            let result=first and not second;'(result)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_live_tuple_elements_keep_binding_and_escape_restrictions() {
    let source = "comptime fn build()-><|-bool> {
        var flag=false;
        inline for (index,pointer) in (flag.ref,flag.ref)'tuple_enumerate {
            pointer=flag.ref;
        }
        '(true)
    }
    fn run()->bool {~<build()>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("immutable"), "{error}");
    let source = "comptime fn build()-><|-*bool> {
        var flag=false;
        inline for (index,pointer) in (flag.ref,flag.ref)'tuple_enumerate {
            return '(pointer);
        }
    }
    fn run()->*bool {~<build()>}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn generator_enum_updates_switch_between_live_and_frozen_payloads() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}
        enum Link {none,some(*bool)}impl Duplicate for Link {}
        comptime fn build()-><|-bool> {
            var flag=false;var slot=Link.none;
            let address=slot.ref;
            slot=Link.some(flag.ref);
            let .some(pointer)=address.* else {return '(false);};
            pointer.* = true;
            slot=Link.none;
            let .none=address.* else {return '(false);};
            '(flag)
        }
        fn run()->bool {~<build()>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("Duplicate"),
            drop: Some("Retire"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generator_aggregate_replacement_keeps_old_pointer_values_and_owner_addresses() {
    let source = "struct Link {pointer:*bool}
        comptime fn build()-><|-bool> {
            var first=false;var second=false;
            var link=Link {pointer:first.ref};
            let old=link.pointer;let address=link.ref;
            link=Link {pointer:second.ref};
            old.* = true;
            address.*.pointer.* = true;
            let result=first and second;'(result)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}
