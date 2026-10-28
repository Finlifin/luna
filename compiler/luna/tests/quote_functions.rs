use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    compile_with_tuple_option(source, emit, false)
}

fn compile_with_tuple_option(source: &str, emit: Emit, tuple: bool) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "quote-functions-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let source = if tuple {
        format!(
            "pure comptime fn TupleElements<T>->.Type'builtin {{T}}
        pure comptime fn Tuple<...Es:TupleElements<.Type'builtin>>->.Type'builtin {{bool}}
        {source}"
        )
    } else {
        source.to_owned()
    };
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile(&Options {
        comptime_intrinsics: Vec::new(),
        input: path.clone(),
        emit,
        output: None,
        build_modes: Default::default(),
        build_targets: Default::default(),
        sysroot: None,
        splice_protocol: None,
        meta_list_type: tuple.then(|| "input.TupleElements".into()),
        tuple_type: tuple.then(|| "input.Tuple".into()),
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        iterator_protocol: None,
        into_iterator_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn quote_functions_retain_their_own_runtime_category() {
    for source in [
        "quote fn twice(value:bool)->bool {value and value}",
        "pure quote fn identity(value:bool)->bool = value;",
        "quote fn add(value:bool,implicit quote current:bool)->bool {value and current}",
        "quote fn declaration(value:bool)->bool;",
    ] {
        let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
            panic!("wrong stage")
        };
        assert!(dump.contains("(quote true)"), "{dump}");
        assert!(dump.contains("(comptime false)"), "{dump}");
        if source.contains("implicit") || source.ends_with("bool;") {
            assert!(compile(source, Emit::Nir).is_err());
        } else {
            let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
                panic!("wrong stage")
            };
            assert!(package.functions.is_empty());
        }
    }
}

#[test]
fn quote_comptime_combinations_cannot_change_function_phase() {
    for source in [
        "quote comptime fn bad()->bool {true}",
        "comptime quote fn bad()->bool {true}",
        "pure quote comptime fn bad()->bool {true}",
    ] {
        assert!(compile(source, Emit::Hir).is_err(), "{source}");
    }
}

#[test]
fn ordinary_and_quote_functions_keep_distinct_modifiers() {
    let Artifact::Hir(dump) = compile(
        "fn ordinary()->bool {true} quote fn quoted()->bool {ordinary()}",
        Emit::Hir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(dump.matches("(quote true)").count(), 1);
    assert_eq!(dump.matches("(quote false)").count(), 1);
}

fn execute(
    source: &str,
    inputs: Vec<comptime_interpretor::Value>,
) -> (comptime_interpretor::Value, Vec<String>) {
    execute_with_tuple_option(source, inputs, false)
}

fn execute_with_tuple_option(
    source: &str,
    inputs: Vec<comptime_interpretor::Value>,
    tuple: bool,
) -> (comptime_interpretor::Value, Vec<String>) {
    let Artifact::Nir(package) = compile_with_tuple_option(source, Emit::Nir, tuple).unwrap()
    else {
        panic!("wrong stage")
    };
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    let value = vm.invoke_instance(&graph.roots[0], inputs).unwrap();
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            comptime_interpretor::Event::EnterFunction(id) => package
                .functions
                .iter()
                .find(|f| f.declaration == *id)
                .map(|f| f.name.clone()),
            _ => None,
        })
        .collect();
    (value, calls)
}

#[test]
fn quote_arguments_evaluate_once_in_source_order_even_when_unused() {
    let (value, calls) = execute(
        "fn first()->bool {true} fn second()->bool {false}
        quote fn duplicate(value:bool)->bool {value and value}
        quote fn ignore(unused:bool)->bool {true}
        fn run()->bool {duplicate(first()) and ignore(second())}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "first", "second"]);
    let (value, calls) = execute(
        "fn first()->bool {true} fn second()->bool {false}
        quote fn select(left:bool,right:bool)->bool {left}
        fn run()->bool {select(first(),second())}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "first", "second"]);
}

#[test]
fn quote_returns_and_repeated_nested_applications_are_hygienic() {
    let source="quote fn identity(value:bool)->bool {let local=value;let callback=|| {return local;};callback()}
        quote fn choose(value:bool)->bool {while true {if value {return identity(value);}break;}false}
        fn run(value:bool)->bool {let local=false;not choose(value) and identity(not value) and not local}";
    for flag in [true, false] {
        let (value, calls) = execute(source, vec![comptime_interpretor::Value::Bool(flag)]);
        assert_eq!(value, comptime_interpretor::Value::Bool(!flag));
        assert!(
            !calls
                .iter()
                .any(|name| name == "choose" || name == "identity")
        );
    }
}

#[test]
fn quote_functions_obey_single_hole_and_cannot_escape_as_runtime_functions() {
    let prefix = "quote fn identity(value:bool)->bool {value} ";
    let (value, _) = execute(
        &format!("{prefix} fn run()->bool {{const code='(identity(true));~<code>}}"),
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    for (source, anchor) in [
        (
            "fn run()->bool {const code='(identity(true) and identity(false));~<code>}",
            "at most one",
        ),
        (
            "fn run()->bool {const first='(true);const code='(identity(~<first>));~<code>}",
            "at most one",
        ),
        (
            "fn run()->bool {let pointer=identity;pointer(true)}",
            "runtime function pointers",
        ),
    ] {
        let failure = compile(&format!("{prefix}{source}"), Emit::Nir)
            .err()
            .unwrap();
        assert!(failure.contains(anchor), "{failure}");
    }
    let failure = compile(
        "quote fn recursive(value:bool)->bool {recursive(value)} fn run()->bool {recursive(true)}",
        Emit::Nir,
    )
    .err()
    .unwrap();
    assert!(failure.contains("expansion depth"), "{failure}");
}

#[test]
fn quote_expansion_cycles_preserve_finite_nested_calls() {
    let source = "quote fn identity(value:bool)->bool {value}
        fn run()->bool {identity(identity(identity(true)))}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    let failure=compile("quote fn first(value:bool)->bool {second(value)} quote fn second(value:bool)->bool {first(value)} fn run()->bool {first(true)}",Emit::Nir).err().unwrap();
    assert!(failure.contains("recursive concrete instance"), "{failure}");
    // A failed cyclic package must not leave expansion state in a later one.
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn quote_expansion_depth_limit_preserves_finite_chains() {
    std::thread::Builder::new()
        .name("quote-depth-boundary".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(check_quote_expansion_depth_limit)
        .unwrap()
        .join()
        .unwrap();
}

fn check_quote_expansion_depth_limit() {
    let mut source = String::from("quote fn q0(value:bool)->bool {value}");
    for index in 1..32 {
        source.push_str(&format!(
            "quote fn q{index}(value:bool)->bool {{q{}(value)}}",
            index - 1
        ));
    }
    let valid = format!("{source}fn run()->bool {{q31(true)}}");
    let nested = (0..32).fold(String::from("true"), |value, _| format!("q0({value})"));
    for valid in [valid, format!("{source}fn run()->bool {{{nested}}}")] {
        let Artifact::Nir(package) = compile(&valid, Emit::Nir).unwrap() else {
            panic!("wrong stage");
        };
        let entry = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap();
        nir::visit::expression(&entry.body, &mut 100_000, |expr, _| {
            if matches!(expr.kind, nir::program::ExprKind::Call { .. }) {
                return Err("unexpanded quote call".into());
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            execute(&valid, vec![]).0,
            comptime_interpretor::Value::Bool(true)
        );
    }
    // A shorter chain also checks the ordinary entry path.
    assert_eq!(
        execute(&format!("{source}fn run()->bool {{q7(true)}}"), vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = format!(
        "{source}quote fn q32(value:bool)->bool {{q31(value)}}fn run()->bool {{q32(true)}}"
    );
    let failure = compile(&invalid, Emit::Nir).err().unwrap();
    assert!(failure.contains("expansion depth"), "{failure}");
}

#[test]
fn quote_templates_keep_phase_purity_and_definition_checks() {
    for (source, anchor) in [
        (
            "pure quote fn write(pointer:*bool) {pointer.* = true;} fn run()->bool {true}",
            "pure",
        ),
        (
            "quote fn identity(value:bool)->bool {value} comptime fn bad()->bool {identity(true)}",
            "phase",
        ),
        ("quote fn bad()->bool {'x'} fn run()->bool {true}", "type"),
        (
            "quote fn q(value:bool)->bool {value} fn accept(comptime x:bool)->bool {x} fn run()->bool {accept(q(true))}",
            "runtime phase",
        ),
        (
            "quote fn id(x:bool)->bool {x} quote fn bad(x:bool)->bool {id(x) and id(x)}",
            "at most one",
        ),
    ] {
        let failure = compile(source, Emit::Nir).err().expect(source);
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generators_can_emit_quote_applications_without_runtime_template_calls() {
    let source = "quote fn negate(value:bool)->bool {not value}
        pure comptime fn build()-><x:bool|-bool> {'(negate(x))}
        fn run(value:bool)->bool {const code=build();~[x=value]<code> and ~[x=value]<code>}";
    let (value, calls) = execute(source, vec![comptime_interpretor::Value::Bool(false)]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn quote_defaults_use_completed_prior_arguments_and_respect_overrides() {
    let source="fn opposite(value:bool)->bool {not value}
        quote fn choose(first:bool,.second:bool=opposite(first),.third:bool=opposite(second))->bool {
            second and not third
        }
        fn run()->bool {choose(false) and choose(true,.second=true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "opposite", "opposite", "opposite"]);
}

#[test]
fn quote_named_inputs_keep_source_order_and_defaults_keep_lexical_identity() {
    let source = "fn first()->bool {true} fn second()->bool {false}
        quote fn choose(value:bool,.option:bool=not value,.result:bool=option)->bool {result}
        fn run()->bool {let value=false;let option=false;
            choose(.option=first(),second()) and choose(false)
        }";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "first", "second"]);
}

#[test]
fn quote_default_suppliers_keep_stability_and_termination_requirements() {
    for (source, anchor) in [
        (
            "fn change(p:*bool)->bool {p.* = true;true} quote fn bad(p:*bool,.x:bool=change(p))->bool {x}",
            "default",
        ),
        (
            "fn recursive()->bool {recursive()} quote fn bad(.x:bool=recursive())->bool {x}",
            "default",
        ),
        (
            "struct Held {flag:bool} fn make()->Held {Held {flag:true}} quote fn bad(.x:bool=make().flag)->bool {x}",
            "default",
        ),
    ] {
        let failure = compile(source, Emit::Nir).err().expect(source);
        assert!(failure.contains(anchor), "{failure}");
    }
    let (value, calls) = execute(
        "quote fn q(x:bool,.y:bool=not x)->bool {not y} pure fn run()->bool {q(true)}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn ordinary_defaults_can_invoke_quote_functions_before_templates_are_erased() {
    let source = "quote fn opposite(value:bool)->bool {not value}
        fn choose(first:bool,.second:bool=opposite(first))->bool {second}
        fn run()->bool {choose(false) and not choose(true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "choose", "choose"]);
}

#[test]
fn generic_quote_functions_instantiate_each_application_and_preserve_returns() {
    let source = "quote fn identity(value:T)->T where T {return value;}
        quote fn wrapped(value:T)->T where T {identity(value)}
        fn run()->bool {wrapped(true) and identity('a') == 'a'}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn generic_callers_keep_result_storage_parameters_until_instantiation() {
    let source = "quote fn identity(value:T)->T where T {value}
        fn generic(value:T)->T where T {identity(value)}
        fn run()->bool {generic(true) and generic('b') == 'b'}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "generic", "generic"]);
}

#[test]
fn quote_generic_requirements_are_discharged_before_expansion() {
    let source = "trait Allowed {} impl Allowed for bool {}
        quote fn identity(value:T)->T where T :- Allowed {value}
        fn run()->bool {identity(true)}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    assert!(
        compile(
            &source.replace("identity(true)", "identity('a')"),
            Emit::Nir
        )
        .is_err()
    );
    let source = "quote fn identity(value:T)->T where T, requires T == bool {value}
        fn run()->bool {identity(true)}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    assert!(
        compile(
            &source.replace("identity(true)", "identity('a')"),
            Emit::Nir
        )
        .is_err()
    );
}

#[test]
fn explicit_generic_quote_applications_count_as_holes() {
    let prefix = "quote fn identity(value:T)->T where T {value} ";
    assert_eq!(
        execute(
            &format!("{prefix} fn run()->bool {{identity<bool>(true)}}"),
            vec![]
        )
        .0,
        comptime_interpretor::Value::Bool(true)
    );
    let failure=compile(&format!("{prefix} fn run()->bool {{const code='(identity<bool>(true) and identity<bool>(false));~<code>}}"),Emit::Nir).err().unwrap();
    assert!(failure.contains("at most one"), "{failure}");
}

#[test]
fn generic_quote_expansion_keeps_opaque_normal_form_results() {
    for body in ["value", "return value;", "if true {return value;}value"] {
        let source = format!(
            "typealias Type=.Type'builtin;
        pure comptime fn Wrap<T> -> Type {{T}}
        quote fn identity(value:T)->T where T {{{body}}}
        fn run(value:Wrap<bool>)->Wrap<bool> {{identity(value)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap();
        assert!(matches!(run.body.ty, nir::Term::NormalForm { .. }));
        for id in &package.internal_return_packets {
            let packet = package
                .enums
                .iter()
                .find(|item| item.declaration == *id)
                .unwrap();
            assert!(matches!(
                packet.variants[1].fields[0],
                nir::Term::NormalForm { .. }
            ));
        }
        assert_eq!(
            package.internal_return_packets.is_empty(),
            !body.starts_with("if")
        );
    }
}

#[test]
fn generic_quote_body_dispatches_trait_methods_after_instantiation() {
    let source = "trait Flag {fn flag(self)->bool;}
        impl Flag for bool {fn flag(self)->bool {self}}
        quote fn read(value:T)->bool where T :- Flag {value.as(Flag).flag()}
        fn run()->bool {read(true)}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn pure_generic_forwarding_keeps_ownership_evidence_after_expansion() {
    let source = "pure quote fn identity(value:T)->T where T {value}
        pure fn forward(value:T)->T where T {identity(value)}
        fn run()->bool {forward(true)}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn quote_type_clause_defaults_are_fixed_and_named_overrides_are_explicit() {
    let source = "quote fn keep(value:T)->T where .T:.Type'builtin=bool {value}
        fn run()->bool {keep(true) and keep<.T=.char'builtin>('a') == 'a'}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
    for call in [
        "keep('a')",
        "keep<bool>(true)",
        "keep<.Missing=bool>(true)",
        "keep<.T=bool,.T=bool>(true)",
    ] {
        let source = format!(
            "quote fn keep(value:T)->T where .T:.Type'builtin=bool {{value}} fn run() {{{call};}}"
        );
        assert!(compile(&source, Emit::Nir).is_err(), "{source}");
    }
}

#[test]
fn quote_type_defaults_depend_on_prior_inferred_and_explicit_types() {
    let source="quote fn pick(first:T,second:U)->U where T,.U:.Type'builtin=T {return second;}
        fn run()->bool {pick(true,false) == false and pick<bool,.U=.char'builtin>(true,'b') == 'b'}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    let source = "quote fn keep(value:U)->U where .T:.Type'builtin=bool,.U:.Type'builtin=T {value}
        fn run()->bool {keep(true) and keep<.T=.char'builtin>('b') == 'b'}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn quote_type_default_definition_errors_and_requirements_cannot_be_erased() {
    for source in [
        "quote fn bad(value:T)->T where .T:.Type'builtin=false {value}",
        "quote fn bad(value:T)->T where .T:.Type'builtin=T {value}",
        "quote fn bad(value:T)->T where .T:.Type'builtin=U,.U:.Type'builtin=bool {value}",
        "quote fn keep(value:T)->T where .T:.Type'builtin=bool,requires T == bool {value} fn run(){keep<.T=.char'builtin>('a');}",
        "quote fn bad(value:bool)->bool where .N:.u8'builtin=256 {value}",
    ] {
        assert!(compile(source, Emit::Nir).is_err(), "{source}");
    }
}

#[test]
fn inherent_quote_methods_expand_receivers_once_and_preserve_aliases() {
    let (value, calls) = execute(
        "struct Flag {value:bool} impl Flag {quote fn get(self)->bool {self.value} quote fn flip(*self) {self.value.* = not self.value.*;}} fn make()->Flag {Flag {value:true}} fn run()->bool {var flag=make();flag.flip();not flag.get()}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "make"]);
    let (value, calls) = execute(
        "struct Flag {value:bool} impl Flag {quote fn get(self)->bool {self.value}} fn make()->Flag {Flag {value:true}} fn run()->bool {make().get()}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "make"]);
}

#[test]
fn inherent_quote_methods_keep_single_hole_and_runtime_phase_rules() {
    let prefix = "struct Flag {value:bool} impl Flag {quote fn get(self)->bool {self.value}} ";
    for (body, anchor) in [
        (
            "fn run()->bool {const code='(Flag {value:true}.get() and Flag {value:false}.get());~<code>}",
            "at most one",
        ),
        (
            "pure comptime fn bad()->bool {Flag {value:true}.get()}",
            "runtime phase",
        ),
    ] {
        let error = compile(&format!("{prefix}{body}"), Emit::Nir)
            .err()
            .expect("must reject");
        assert!(error.contains(anchor), "{error}");
    }
}

#[test]
fn generic_inherent_quote_methods_instantiate_container_and_method_types() {
    let (value, calls) = execute(
        "struct Holder where T {value:T} impl Holder<T> where T {quote fn take(self)->T {self.value} quote fn choose(self,other:U)->U where U {other}} fn forward(value:T)->T where T {Holder<T> {value:value}.take()} fn run()->bool {forward(true) and Holder<bool> {value:false}.choose('a') == 'a'}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward"]);
}

#[test]
fn inherent_quote_method_defaults_observe_bound_receiver() {
    let (value, calls) = execute(
        "struct Flag {value:bool} impl Flag {quote fn get(self,.answer:bool=self.value)->bool {answer}} fn make()->Flag {Flag {value:true}} fn run()->bool {make().get() and not make().get(.answer=false)}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "make", "make"]);
}

#[test]
fn quote_methods_declared_inside_nominals_expand_without_runtime_entrypoints() {
    let (value, calls) = execute(
        "struct Flag {value:bool, quote fn get(self)->bool {self.value}} enum Choice {yes,no, quote fn get(self)->bool {self match {.yes=>true,.no=>false}}} fn run()->bool {Flag {value:true}.get() and Choice.yes.get() and not Choice.no.get()}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn generic_quote_methods_cannot_bypass_hole_or_stage_checks() {
    let prefix = "struct Holder where T {value:T} impl Holder<T> where T {quote fn take(self)->T {self.value}} ";
    for (body, anchor) in [
        (
            "fn run()->bool {const code='(Holder<bool> {value:true}.take() and Holder<bool> {value:false}.take());~<code>}",
            "at most one",
        ),
        (
            "pure comptime fn bad()->bool {Holder<bool> {value:true}.take()}",
            "runtime phase",
        ),
    ] {
        let error = compile(&format!("{prefix}{body}"), Emit::Nir)
            .err()
            .expect("must reject");
        assert!(error.contains(anchor), "{error}");
    }
    assert!(compile(&format!("{prefix} fn run()->bool {{let action=Holder<bool>.take;action(Holder<bool> {{value:true}})}}"),Emit::Nir).is_err());
    let (value, calls) = execute(
        "trait Read {quote fn read(self)->bool {true}} struct Flag {} impl Read for Flag {} fn run()->bool {Flag {}.as(Read).read()}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn nested_callable_modifiers_are_preserved_and_checked() {
    for source in [
        "struct Flag {pure quote fn bad(pointer:*bool) {pointer.* = true;}}",
        "enum Choice {yes, pure quote fn bad(pointer:*bool) {pointer.* = true;}}",
        "struct Flag {quote comptime fn bad()->bool {true}}",
        "enum Choice {yes, quote comptime fn bad()->bool {true}}",
    ] {
        assert!(compile(source, Emit::Nir).is_err(), "{source}");
    }
    let (value, calls) = execute(
        "struct Flag {pub pure quote fn get()->bool {true}} fn run()->bool {Flag.get()}",
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn quote_variadics_collect_expand_and_forward_complete_packets() {
    let source = "quote fn pack(...items:T)->T where T {return items;}
        fn packet()->Tuple<bool,.char'builtin> {(true,'a')}
        fn run()->bool {
            let (a,b,c)=pack(...packet(),false);
            let (d,e)=pack(.items=(true,'a'));
            pack();
            a and b == 'a' and not c and d and e == 'a'
        }";
    let (value, calls) = execute_with_tuple_option(source, vec![], true);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "packet"]);
}

#[test]
fn quote_variadic_properties_preserve_interleaved_producers() {
    let source = "fn packet()->Tuple<bool,bool> {(true,false)}
        fn before()->bool {true} fn after()->bool {false}
        quote fn configure(...items:Tuple<bool,bool>,...properties:T)->T where T {properties}
        fn run()->bool {
            let config=configure {first:before(),...packet(),last:after()};
            config.first and not config.last
        }";
    let (value, calls) = execute_with_tuple_option(source, vec![], true);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "before", "packet", "after"]);
}

#[test]
fn quote_variadic_methods_evaluate_receiver_before_packets() {
    let source = "struct Flag {value:bool}
        impl Flag {quote fn choose(self,...items:Tuple<bool,bool>)->bool {
            let (a,b)=items;if self.value {return a;} b
        }}
        fn receiver()->Flag {Flag {value:true}}
        fn packet()->Tuple<bool,bool> {(true,false)}
        fn run()->bool {receiver().choose(...packet())}";
    let (value, calls) = execute_with_tuple_option(source, vec![], true);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "receiver", "packet"]);
}

#[test]
fn quote_variadics_keep_default_dependencies_and_argument_context() {
    let source = "pure fn first(items:Tuple<bool,bool>)->bool {let (a,b)=items;a}
        quote fn choose(...items:Tuple<bool,bool>,.flag:bool=first(items))->bool {
            let (a,b)=items;if flag {return a;} b
        }
        fn run()->bool {choose(true,false) and not choose(true,false,.flag=false)}";
    let (value, calls) = execute_with_tuple_option(source, vec![], true);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "first"]);
}

#[test]
fn quote_variadic_calls_still_enforce_phase_and_single_hole() {
    let prefix = "quote fn pack(...items:T)->T where T {items} ";
    for (body, anchor) in [
        (
            "comptime fn bad()->Tuple<bool,bool> {pack(true,false)}",
            "runtime phase",
        ),
        (
            "fn run(){const c='(do {pack(true);pack(false);});}",
            "at most one",
        ),
        (
            "fn run(){const c='(do {pack {a:true};pack {b:false};});}",
            "at most one",
        ),
        ("fn run(){pack {a:true,a:false};}", "more than once"),
    ] {
        let failure = compile_with_tuple_option(&format!("{prefix}{body}"), Emit::Nir, true)
            .err()
            .unwrap();
        assert!(failure.contains(anchor), "{failure}");
    }
}

#[test]
fn quote_variadic_empty_spreads_run_between_properties() {
    let source =
        "fn empty(...items:T)->T where T {items} fn first()->bool {true} fn last()->bool {false}
        quote fn config(...properties:T)->T where T {properties}
        fn run()->bool {
            let settings=config {before:first(),...empty(),after:last()};
            settings.before and not settings.after
        }";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "first", "empty", "last"]);
}

#[test]
fn trait_quote_declarations_retain_the_required_code_category() {
    let Artifact::Nir(package) = compile(
        "trait Access {quote fn read(self)->bool;} fn run()->bool {true}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    let interface = package
        .traits
        .iter()
        .find(|interface| interface.name == "Access")
        .unwrap();
    let method = &interface.methods[0];
    assert!(method.is_quote);
    assert!(method.to_string().contains("(quote true)"));
    assert!(!method.signature.is_comptime_callable());
    assert!(
        !package
            .functions
            .iter()
            .any(|function| function.declaration == method.declaration)
    );
}

#[test]
fn ordinary_trait_implementations_cannot_erase_required_quote_expansion() {
    let failure = compile(
        "trait Access {quote fn read(self)->bool;}
        struct Flag {} impl Access for Flag {fn read(self)->bool {true}}",
        Emit::Nir,
    )
    .err()
    .unwrap();
    assert!(
        failure.contains("requires a quote fn implementation"),
        "{failure}"
    );
}

#[test]
fn materialization_never_turns_a_quote_contract_into_runtime_dispatch() {
    let Artifact::Nir(mut package) = compile(
        "trait Access {fn read(self)->bool;}
        struct Flag {} impl Access for Flag {fn read(self)->bool {true}}
        fn run()->bool {Flag {}.as(Access).read()}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    // Exercise the serialized-IR boundary independently of source admission:
    // a retained quote contract is never permission to execute an ordinary body.
    package
        .traits
        .iter_mut()
        .find(|interface| interface.name == "Access")
        .unwrap()
        .methods[0]
        .is_quote = true;
    let failure = luna::instantiation::instantiate(&package, "run", 100_000).unwrap_err();
    assert!(
        failure.contains("runtime dispatch is not permitted"),
        "{failure}"
    );
}

#[test]
fn quote_templates_survive_nir_separately_from_executable_functions() {
    let source = "quote fn identity(value:T)->T where T {value}
        quote fn nested(value:bool)->bool {identity(value)}
        fn run()->bool {nested(true)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(package.quote_templates.len(), 2);
    assert_eq!(
        package
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["run"]
    );
    let generic = package
        .quote_templates
        .iter()
        .find(|f| f.name == "identity")
        .unwrap();
    assert_eq!(generic.type_parameters.len(), 1);
    assert_eq!(generic.parameters.len(), 1);
    nir::phases::validate(&package).unwrap();
    assert!(luna::instantiation::instantiate(&package, "identity", 100_000).is_err());
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(!graph.functions.iter().any(|instance| {
        package
            .quote_templates
            .iter()
            .any(|template| template.declaration == instance.key.declaration)
    }));
}

#[test]
fn concrete_trait_quote_implementations_expand_after_selection() {
    let source = "trait Read {quote fn read(self)->bool;}
        struct Flag {value:bool}
        impl Read for Flag {quote fn read(self)->bool {return self.value;}}
        fn make()->Flag {Flag {value:true}}
        fn run()->bool {make().as(Read).read()}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "make"]);
}

#[test]
fn trait_quote_defaults_are_rebound_to_implementation_parameters() {
    let source = "trait Select {quote fn choose(self,first:bool,.second:bool=not first)->bool;}
        struct Flag {}
        impl Select for Flag {quote fn choose(self,value:bool,.answer:bool=true)->bool {answer}}
        fn mark()->bool {true}
        fn run()->bool {not Flag {}.as(Select).choose(mark())}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "mark"]);
}

#[test]
fn generic_trait_quote_implementation_substitutes_subject_and_method_arguments() {
    let source = "trait Identity {quote fn keep(self,value:U)->U where U;}
        struct Holder where T {value:T}
        impl Identity for Holder<T> where T {
            quote fn keep(self,value:U)->U where U {return value;}
        }
        fn run()->bool {Holder<bool> {value:false}.as(Identity).keep(true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn trait_quote_applications_keep_holes_phase_and_escape_checks() {
    let prefix =
        "trait Read {quote fn read(self)->bool {true}} struct Flag {} impl Read for Flag {} ";
    for (body, anchor) in [
        (
            "fn run(){const code='(Flag {}.as(Read).read() and Flag {}.as(Read).read());}",
            "at most one",
        ),
        (
            "comptime fn bad()->bool {Flag {}.as(Read).read()}",
            "runtime phase",
        ),
    ] {
        let failure = compile(&format!("{prefix}{body}"), Emit::Nir)
            .err()
            .unwrap();
        assert!(failure.contains(anchor), "{failure}");
    }
}

#[test]
fn generic_runtime_callers_expand_trait_quote_methods_at_materialization() {
    let source = "trait Read {quote fn read(self)->bool;}
        struct Yes {} struct No {}
        impl Read for Yes {quote fn read(self)->bool {return true;}}
        impl Read for No {quote fn read(self)->bool {false}}
        fn read(value:T)->bool where T :- Read {value.as(Read).read()}
        fn run()->bool {read(Yes {}) and not read(No {})}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "read", "read"]);
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(!graph.functions.iter().any(|function| {
        package
            .quote_templates
            .iter()
            .any(|template| template.declaration == function.key.declaration)
    }));
    assert!(graph.expansion_sources.len() > package.expansion_sources.len());
}

#[test]
fn late_trait_quote_defaults_and_reference_receivers_preserve_binding_identity() {
    let source = "trait Flip {quote fn flip(*self,.flag:bool=true)->bool;}
        struct Flag {value:bool}
        impl Flip for Flag {quote fn flip(*self,.replacement:bool=false)->bool {
            self.value.* = replacement;self.value.*
        }}
        fn flip(value:*T)->bool where T :- Flip {value.as(Flip).flip()}
        fn run()->bool {var value=Flag {value:false};flip(value.ref) and value.value}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "flip"]);
}

#[test]
fn recursive_trait_quote_expansion_is_bounded_after_generic_instantiation() {
    let source = "trait Read {quote fn read(self)->bool;}
        struct Flag {}
        impl Read for Flag {quote fn read(self)->bool {self.as(Read).read()}}
        fn read(value:T)->bool where T :- Read {value.as(Read).read()}
        fn run()->bool {read(Flag {})}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let failure = luna::instantiation::instantiate(&package, "run", 100_000).unwrap_err();
    assert!(
        failure.contains("expansion depth") || failure.contains("budget"),
        "{failure}"
    );
}

#[test]
fn quote_parameter_protocols_route_lambda_syntax_and_generic_inputs() {
    for source in [
        "quote fn apply(lambda action:fn(bool)->bool,value:bool)->bool {return action(value);} fn run()->bool {apply(_0,true)}",
        "quote fn apply(lambda action:fn(T)->T,value:T)->T where T {action(value)} fn run()->bool {apply(_0,...(true,))}",
        "quote fn apply(lambda action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply() do {_0}}",
        "quote fn apply(lambda action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply() do |value| {value}}",
        "quote fn apply(lambda action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply(do {let pointer=_0.ref;pointer.* = false;not _0})}",
        "quote fn outer(lambda action:fn(bool)->bool)->bool {action(true)} quote fn inner(lambda action:fn(bool)->bool)->bool {action(false)} fn run()->bool {outer(_0 and not inner(_0))}",
    ] {
        let (value, calls) = execute(source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true), "{source}");
        assert_eq!(calls, ["run"], "{source}");
    }
}

#[test]
fn quote_parameter_protocols_preserve_explicit_implicit_supply_and_source_order() {
    let source = "fn first()->bool {true} fn second()->bool {false}
        quote fn apply(value:bool,implicit enabled:bool,lambda action:fn(bool)->bool)->bool {
            enabled and action(value)
        }
        fn run()->bool {apply(.enabled=first(),second(),not _0)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "first", "second"]);
}

#[test]
fn quote_parameter_protocols_capture_after_extended_property_evaluation() {
    let source = "comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {enabled:true};typealias Options=SAMPLE'type;
        fn mark(p:*bool)->bool {p.* = true;true}
        quote fn apply(lambda action:%{flag:bool} fn()->bool,...properties:Options)->bool {
            properties.enabled and action()
        }
        fn run()->bool {let flag=false;apply {enabled:mark(flag.ref)} do {flag}}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "mark"]);
}

#[test]
fn quote_parameter_protocols_survive_trait_selection_and_late_expansion() {
    let source = "trait Apply {
            quote fn apply(self,lambda action:fn(bool)->bool,implicit enabled:bool)->bool;
        }
        struct Host {}
        impl Apply for Host {
            quote fn apply(self,action:fn(bool)->bool,enabled:bool)->bool {action(enabled)}
        }
        fn invoke(host:T)->bool where T :- Apply {
            host.as(Apply).apply(.enabled=true) do {_0}
        }
        fn run()->bool {invoke(Host {}) and Host {}.as(Apply).apply(_0,.enabled=true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "invoke"]);
}

#[test]
fn quote_parameter_protocols_reject_missing_ambiguous_and_invalid_supplies() {
    for (source, anchor) in [
        (
            "quote fn apply(implicit value:bool)->bool {value} fn run()->bool {apply()}",
            "automatic supply protocol",
        ),
        (
            "quote fn apply(implicit value:bool)->bool {value} fn run()->bool {apply(true)}",
            "too many positional",
        ),
        (
            "quote fn apply(implicit value:bool)->bool {value} fn run()->bool {apply(.value=true,.value=false)}",
            "parameter supplied more than once",
        ),
        (
            "quote fn apply(lambda action:fn(bool)->bool)->bool {action(true)} fn run()->bool {apply(_1)}",
            "exceeds target arity",
        ),
        (
            "quote fn apply(lambda first:fn()->bool,lambda second:fn()->bool) {} fn run() {apply() do {true};}",
            "ambiguous post do",
        ),
        (
            "quote fn apply(implicit quote value:bool)->bool {value}",
            "contextual quote fn inputs",
        ),
    ] {
        let failure = compile(source, Emit::Nir)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(failure.contains(anchor), "{failure}");
    }
}

#[test]
fn staged_quote_inputs_complete_before_runtime_arguments_and_never_emit_calls() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        fn produce()->bool {true}
        quote fn apply(comptime enabled:bool,value:bool)->bool {if enabled {return value;}false}
        fn run()->bool {apply(invert(false),produce()) and not apply(false,produce())}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "produce", "produce"]);
}

#[test]
fn staged_quote_defaults_and_nested_forwarding_use_completed_inputs() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        quote fn inner(comptime flag:bool,value:bool)->bool {flag and value}
        quote fn outer(comptime flag:bool,value:bool,comptime .other:bool=invert(flag))->bool {
            inner(other,value)
        }
        fn forward(comptime flag:bool,value:bool)->bool {outer(flag,value)}
        fn run()->bool {forward(false,true) and not forward(true,true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward", "forward"]);
}

#[test]
fn staged_quote_type_inputs_specialize_dependent_domains() {
    let source = "quote fn identity(comptime T:.Type'builtin,value:T)->T {value}
        fn run()->bool {identity(bool,true) and identity(.char'builtin,'a') == 'a'}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn staged_quote_trait_methods_select_then_bind_interface_defaults() {
    let source="trait Pick {quote fn pick(self,comptime .flag:bool=true)->bool;}
        struct Host {}
        impl Pick for Host {quote fn pick(self,comptime .other:bool=false)->bool {other}}
        fn forward(value:T,comptime flag:bool)->bool where T :- Pick {value.as(Pick).pick(.flag=flag)}
        fn run()->bool {Host {}.as(Pick).pick() and forward(Host {},true) and not forward(Host {},false)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward", "forward"]);
}

#[test]
fn staged_quote_inputs_do_not_read_future_runtime_values() {
    for source in [
        "quote fn apply(comptime flag:bool)->bool {flag} fn run(flag:bool)->bool {apply(flag)}",
        "fn produce()->bool {true} quote fn apply(comptime flag:bool)->bool {flag} fn run()->bool {apply(produce())}",
    ] {
        assert!(compile(source, Emit::Nir).is_err(), "{source}");
    }
}

#[test]
fn staged_quote_requirements_are_checked_after_input_substitution() {
    let prefix="quote fn restricted(comptime T:.Type'builtin,value:bool)->bool where requires T == bool {value}
        fn forward(comptime T:.Type'builtin)->bool where requires T == bool {restricted(T,true)} ";
    let (value, calls) = execute(
        &format!("{prefix} fn run()->bool {{forward(bool)}}"),
        vec![],
    );
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward"]);
    let source = format!("{prefix} fn run()->bool {{forward(.char'builtin)}}");
    match compile(&source, Emit::Nir) {
        Err(failure) => assert!(
            failure.contains("requirement") || failure.contains("constraint"),
            "{failure}"
        ),
        Ok(Artifact::Nir(package)) => {
            assert!(luna::instantiation::instantiate(&package, "run", 100_000).is_err())
        }
        _ => panic!("wrong stage"),
    }
}

#[test]
fn staged_quote_recursion_exhausts_budget_without_runtime_fallback() {
    let Artifact::Nir(package) = compile(
        "quote fn recursive(comptime flag:bool)->bool {recursive(flag)}
        fn run()->bool {recursive(true)}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    let failure = luna::instantiation::instantiate(&package, "run", 3_000)
        .err()
        .unwrap();
    assert!(
        failure.contains("budget") || failure.contains("exhausted"),
        "{failure}"
    );
    // A failed expansion must not mutate the retained templates.
    let failure = luna::instantiation::instantiate(&package, "run", 3_000)
        .err()
        .unwrap();
    assert!(
        failure.contains("budget") || failure.contains("exhausted"),
        "{failure}"
    );
}

#[test]
fn staged_quote_materialization_rechecks_retained_type_obligations() {
    let Artifact::Nir(mut package) = compile(
        "quote fn choose(comptime flag:bool)->bool {flag}
        fn run()->bool {choose(true)}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    package.quote_templates[0]
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    let failure = luna::instantiation::instantiate(&package, "run", 100_000)
        .err()
        .unwrap();
    assert!(
        failure.contains("quote function type requirement"),
        "{failure}"
    );
}

#[test]
fn staged_quote_templates_remain_descriptive_until_materialization() {
    let Artifact::Nir(package) = compile(
        "quote fn apply(comptime flag:bool,value:bool)->bool {flag and value}
        fn run()->bool {apply(true,false)}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    let template = package
        .quote_templates
        .iter()
        .find(|template| template.name == "apply")
        .unwrap();
    assert!(
        !package
            .functions
            .iter()
            .any(|function| function.declaration == template.declaration)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        !graph
            .functions
            .iter()
            .any(|function| function.key.declaration == template.declaration)
    );
    assert!(graph.expansion_sources.len() > package.expansion_sources.len());
    for function in &graph.functions {
        nir::visit::expressions(&function.function,&mut 10_000,|expr,_| {
            assert!(!matches!(&expr.kind,nir::program::ExprKind::Function(id)
                | nir::program::ExprKind::FunctionInstance {declaration:id,..} if *id==template.declaration));
            Ok(())
        }).unwrap();
    }
}

#[test]
fn staged_quote_trait_type_requirements_wait_for_ordinary_type_inputs() {
    let source="trait Identity {
            quote fn keep(self,comptime T:.Type'builtin,value:T)->T where requires T == bool;
        }
        struct Host {}
        impl Identity for Host {
            quote fn keep(self,comptime T:.Type'builtin,value:T)->T where requires T == bool {value}
        }
        fn forward(host:X,comptime T:.Type'builtin,value:T)->T where X :- Identity, requires T == bool {
            host.as(Identity).keep(T,value)
        }
        fn run()->bool {Host {}.as(Identity).keep(bool,true) and forward(Host {},bool,true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward"]);
}

#[test]
fn contracted_quote_checks_old_values_and_logical_return_results() {
    let source = "quote fn flip(value:bool)->result:bool
            where requires value, ensures result == not value'old {
            value = not value;return value;
        }
        fn run()->bool {not flip(true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn contracted_quote_rejects_unproven_preconditions_and_postconditions() {
    for source in [
        "quote fn checked(value:bool)->bool where requires value {value} fn run()->bool {checked(false)}",
        "quote fn checked(value:bool)->result:bool where ensures result {value} fn run()->bool {checked(false)}",
        "quote fn unused()->result:bool where ensures result {false} fn run()->bool {true}",
    ] {
        let failure = compile(source, Emit::Nir).err().unwrap();
        assert!(failure.contains("not proven"), "{failure}");
    }
}

#[test]
fn contracted_staged_quote_preserves_default_inputs_and_old_values() {
    let source = "quote fn checked(comptime flag:bool,.value:bool=flag)->result:bool
            where requires value == flag, ensures result == flag'old {
            return value;
        }
        fn run()->bool {checked(true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn contracted_quote_trait_interfaces_retain_checks_when_implementation_is_weaker() {
    let source = "trait Check {quote fn check(self,value:bool)->bool where requires value;}
        struct Host {} impl Check for Host {quote fn check(self,value:bool)->bool {value}}
        fn forward(host:T,value:bool)->bool where T :- Check {host.as(Check).check(value)}
        fn run()->bool {forward(Host {},false)}";
    let failure = compile(source, Emit::Nir).err().unwrap();
    assert!(failure.contains("precondition is not proven"), "{failure}");
}

#[test]
fn contracted_quote_trait_results_and_snapshots_use_implementation_bindings() {
    let source="trait Flip {
            quote fn flip(self,value:bool)->result:bool where requires value, ensures result == not value'old;
        }
        struct Host {}
        impl Flip for Host {
            quote fn flip(self,input:bool)->answer:bool where requires input, ensures answer == not input'old {
                input = not input;return input;
            }
        }
        fn forward(host:T)->bool where T :- Flip {host.as(Flip).flip(true)}
        fn run()->bool {not Host {}.as(Flip).flip(true) and not forward(Host {})}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward"]);
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let dump = package.to_string();
    for node in ["(snapshot ", "(require-predicate ", "(ensure-predicate "] {
        assert!(dump.contains(node), "missing {node}");
    }
}

#[test]
fn contracted_quote_checks_each_exit_but_not_nested_closure_returns() {
    let source = "quote fn checked(flag:bool)->result:bool where ensures result {
            let callback=|| {return false;};
            let ignored=callback();
            if flag {return true;}
            true
        }
        fn run(flag:bool)->bool {checked(flag)}";
    for flag in [false, true] {
        let (value, calls) = execute(source, vec![comptime_interpretor::Value::Bool(flag)]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true));
        assert_eq!(calls, ["run"]);
    }
    let failure = compile(
        "quote fn checked(flag:bool)->result:bool where ensures result {
        if flag {return false;}true
    } fn run(flag:bool)->bool {checked(flag)}",
        Emit::Nir,
    )
    .err()
    .unwrap();
    assert!(failure.contains("postcondition is not proven"), "{failure}");
}

#[test]
fn contracted_quote_unproven_callback_result_reports_without_exhausting_stack() {
    let failure = compile(
        "quote fn checked()->result:bool where ensures result {
        let callback=|| {return false;};return not callback();
    } fn run()->bool {checked()}",
        Emit::Nir,
    )
    .err()
    .unwrap();
    assert!(failure.contains("postcondition is not proven"), "{failure}");
}

#[test]
fn effectful_quote_expands_body_and_preserves_handler_resume_and_return() {
    for (body, expected) in [
        ("subject(true)# {read()=>resume false,}", true),
        ("subject(false)# {read()=>return true,};false", true),
        ("subject(false)# {read()=>resume true,}", false),
    ] {
        let source = format!(
            "effect read()->bool;
            quote fn subject(value:bool)->#read bool {{read()#;return value;}}
            fn run()->bool {{{body}}}"
        );
        let (value, calls) = execute(&source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(expected));
        assert_eq!(calls, ["run"]);
    }
}

#[test]
fn effectful_quote_arguments_run_in_caller_environment_before_body() {
    let source = "effect extra()->bool;effect read()->bool;
        quote fn subject(value:bool)->#read bool {value and not (read()#)}
        fn outer()->#[extra,read] bool {subject(extra()#)}
        fn run()->bool {outer()# {extra()=>true,read()=>false,}}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "outer"]);
}

#[test]
fn effectful_staged_and_trait_quotes_keep_computation_rows() {
    let source = "effect read()->bool;
        trait Read {quote fn receive(self,comptime enabled:bool)->#read bool;}
        struct Host {}
        impl Read for Host {
            quote fn receive(self,comptime enabled:bool)->#read bool {enabled and (read()#)}
        }
        fn forward(host:T)->#read bool where T :- Read {host.as(Read).receive(true)}
        fn run()->bool {forward(Host {})# {read()=>true,}}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward"]);
}

#[test]
fn effectful_quote_direct_passing_keeps_installation_before_argument_evaluation() {
    let source = "^attrs.effects.direct_passing effect setting()->bool;
        fn argument()->bool {true}
        fn provide()->bool {true}
        quote fn subject(value:bool)->#setting bool {value and (setting()#) and (setting()#)}
        fn run()->bool {subject(argument())# {setting()=>provide(),}}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "provide", "argument"]);
}

#[test]
fn effectful_quote_affine_direct_passing_slots_keep_request_counts() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);} struct Token {}
        ^attrs.effects.direct_passing effect token()->Token;";
    for (body, allowed) in [
        ("if flag {return token()#;}token()#", true),
        ("let ignored=token()#;token()#", false),
    ] {
        let source = format!(
            "{prefix} quote fn subject(flag:bool)->#token Token {{{body}}}
            fn run()->Token {{subject(true)# {{token()=>Token {{}},}}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_effects_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if allowed {
            assert!(matches!(
                result.unwrap(),
                comptime_interpretor::Value::Record { .. }
            ));
        } else {
            let failure = result.unwrap_err();
            assert!(failure.contains("affine effect slot"), "{failure}");
        }
    }
}

#[test]
fn effectful_quote_handler_returns_target_the_quote_application() {
    let source = "effect read()->bool;
        quote fn subject()->#read bool {
            read()# {read()=>return true,};false
        }
        fn run()->bool {not (subject()# {read()=>false,})}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(false));
    assert_eq!(calls, ["run"]);
}

#[test]
fn effectful_quotes_still_require_matching_effect_environments() {
    for source in [
        "effect read()->bool; quote fn subject()->#read bool {read()#} fn run()->bool {subject()#}",
        "effect read()->bool;effect other()->bool; quote fn subject()->#read bool {read()#} fn run()->bool {subject()# {other()=>true,}}",
        "effect read()->bool; quote fn subject()->#read bool {read()#} comptime fn bad()->bool {subject()# {read()=>true,}}",
    ] {
        assert!(compile(source, Emit::Nir).is_err(), "accepted {source}");
    }
}

#[test]
fn effect_result_contracts_bind_completed_values_for_ordinary_and_quote_functions() {
    for modifier in ["", "quote "] {
        let source = format!(
            "effect tick()->bool;
            {modifier}fn subject(flag:bool)->result:#tick bool where ensures result == flag'old {{
                tick()#;return flag;
            }}
            fn run()->result:bool where ensures result {{subject(true)# {{tick()=>false,}}}}"
        );
        let (value, calls) = execute(&source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true));
        assert_eq!(
            calls,
            if modifier.is_empty() {
                vec!["run", "subject"]
            } else {
                vec!["run"]
            }
        );
    }
}

#[test]
fn effect_result_contracts_do_not_preserve_handler_mutated_lexical_facts() {
    let source = "effect tick()->bool;quote fn subject()->#tick bool {tick()#}
        fn run()->result:bool where ensures result {
            var flag=true;
            subject()# {tick()=>{flag=false;true},};
            flag
        }";
    let failure = compile(source, Emit::Nir).err().unwrap();
    assert!(failure.contains("postcondition is not proven"), "{failure}");
}

#[test]
fn effect_result_contracts_compose_staged_trait_methods_and_generic_callers() {
    let source = "effect poll()->bool;
        trait Decide {quote fn decide(self,comptime expected:bool)->result:#poll bool
            where ensures result == expected'old;}
        struct Host {}
        impl Decide for Host {quote fn decide(self,comptime value:bool)->answer:#poll bool
            where ensures answer == value'old {poll()#;return value;}}
        fn forward(host:T)->result:#poll bool where T :- Decide, ensures result {
            host.as(Decide).decide(true)
        }
        fn run()->result:bool where ensures result {forward(Host {})# {poll()=>false,}}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "forward"]);
}

#[test]
fn effect_result_contracts_preserve_completed_aggregate_facts() {
    for modifier in ["", "quote "] {
        let source = format!(
            "effect poll()->bool;struct Packet {{flag:bool}}
            {modifier}fn subject()->result:#poll Packet where ensures result.flag {{
                poll()#;Packet {{flag:true}}
            }}
            fn run()->result:bool where ensures result {{
                let packet=subject()# {{poll()=>false,}};packet.flag
            }}"
        );
        let (value, _) = execute(&source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true));
    }
}

#[test]
fn effect_result_contracts_require_proof_on_each_normal_completion() {
    for modifier in ["", "quote "] {
        let source = format!(
            "effect poll()->bool;
            {modifier}fn bad(flag:bool)->result:#poll bool where ensures result {{
                poll()#;if flag {{return false;}}true
            }} fn run()->bool {{true}}"
        );
        let failure = compile(&source, Emit::Nir).err().unwrap();
        assert!(failure.contains("postcondition is not proven"), "{failure}");
    }
}

#[test]
fn effect_result_contracts_keep_completed_inputs_separate_from_handler_writes() {
    for modifier in ["", "quote "] {
        let source=format!("effect poll()->bool;
            {modifier}fn subject(flag:bool)->result:#poll bool where ensures result == flag'old {{poll()#;flag}}
            fn run()->result:bool where ensures result {{
                var flag=true;
                let completed=subject(flag)# {{poll()=>{{flag=false;true}},}};
                assumes not flag;
                completed and not flag
            }}");
        let (value, _) = execute(&source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true));
    }
}

#[test]
fn effect_result_contracts_validate_handler_escapes_against_the_caller() {
    let prefix = "effect poll()->bool;quote fn subject()->result:#poll bool where ensures result {poll()#;true}";
    let source = format!(
        "{prefix} fn run()->result:bool where ensures result {{subject()# {{poll()=>return true,}}}}"
    );
    let (value, _) = execute(&source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    let source = format!(
        "{prefix} fn run()->result:bool where ensures result {{subject()# {{poll()=>return false,}}}}"
    );
    let failure = compile(&source, Emit::Nir).err().unwrap();
    assert!(failure.contains("postcondition is not proven"), "{failure}");
}

#[test]
fn effect_request_diverging_argument_returns_before_dispatch_for_ordinary_and_quote_functions() {
    for modifier in ["", "quote "] {
        let source = format!(
            "effect send(first:bool,second:bool)->bool;
            fn argument()->bool {{true}}
            fn handler()->bool {{false}}
            {modifier}fn subject()->#send bool {{send(argument(),do {{return true;}})#}}
            fn run()->bool {{subject()# {{send(first,second)=>handler(),}}}}"
        );
        let (value, calls) = execute(&source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true));
        assert_eq!(
            calls,
            if modifier.is_empty() {
                vec!["run", "subject", "argument"]
            } else {
                vec!["run", "argument"]
            }
        );
    }
}

#[test]
fn quote_explicit_type_clause_domains_use_ordinary_instantiation() {
    let source = "typealias Kind=.Type'builtin;
        trait Flag {fn flag(self)->bool;} impl Flag for bool {fn flag(self)->bool {self}}
        quote fn keep(value:T)->T where T:Kind {value}
        quote fn pick(first:T,second:U)->U where T:Kind,.U:Kind=T {second}
        quote fn flag(value:T)->bool where T:Kind, requires T :- Flag {value.as(Flag).flag()}
        fn run()->bool {keep(true) and keep<.char'builtin>('a') == 'a'
            and not pick(true,false) and pick<bool,.U=.char'builtin>(true,'b') == 'b'
            and flag(true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "flag"]);
}

#[test]
fn quote_clause_domains_reject_invalid_values_and_failed_constraints() {
    for source in [
        "quote fn invalid()->bool where value:bool {value} fn run()->bool {invalid<'x'>()}",
        "quote fn keep(value:T)->T where T:.Type'builtin {value} fn run()->bool {keep<bool>('x')}",
        "trait Flag {} quote fn keep(value:T)->T where T:.Type'builtin, requires T :- Flag {value} fn run()->bool {keep(true)}",
    ] {
        let failure = compile(source, Emit::Nir)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(
            failure.contains("elaboration failed"),
            "{source}: {failure}"
        );
    }
}

#[test]
fn quote_methods_capture_nominal_value_parameters() {
    let source = "struct Switch where enabled:bool {
        pub quote fn choose(value:bool)->bool {enabled and value}
    }
    fn run()->bool {Switch<true>.choose(true) and not Switch<false>.choose(true)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn quote_methods_preserve_dependent_values_and_single_argument_evaluation() {
    let source = "struct Holder where T,value:T {
        pub quote fn read(unused:bool)->T {value}
    }
    fn input()->bool {true}
    fn run()->bool {Holder<bool,true>.read(input()) and Holder<.u8'builtin,201>.read(input()) == 201}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "input", "input"]);
    let invalid = source.replace(".u8'builtin,201", ".u8'builtin,256");
    assert!(compile(&invalid, Emit::Nir).is_err());
}

#[test]
fn quote_trait_defaults_capture_value_indices_without_runtime_dispatch() {
    let source = "trait Select where enabled:bool {
        quote fn choose(value:bool)->bool {enabled and value}
    }
    struct Switch {} impl Select<true> for Switch {} impl Select<false> for Switch {}
    fn input()->bool {true}
    fn run()->bool {Switch.as(Select<true>).choose(input()) and not Switch.as(Select<false>).choose(input())}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run", "input", "input"]);
}

#[test]
fn quote_value_indexed_interface_contracts_survive_implementation_expansion() {
    let source = "trait Select where enabled:bool {
        quote fn choose(value:bool)->result:bool where requires value == enabled, ensures result == enabled;
    }
    struct Switch {}
    impl Select<true> for Switch {quote fn choose(value:bool)->bool {value}}
    impl Select<false> for Switch {quote fn choose(value:bool)->bool {value}}
    fn run()->bool {Switch.as(Select<true>).choose(true) and not Switch.as(Select<false>).choose(false)}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
    for invalid in [
        source.replace("choose(true)", "choose(false)"),
        source.replace("->bool {value}", "->bool {not value}"),
    ] {
        let failure = compile(&invalid, Emit::Nir)
            .err()
            .expect("invalid contract accepted");
        assert!(failure.contains("not proven"), "{failure}");
    }
}

#[test]
fn quote_integer_value_contracts_prove_completed_bounds() {
    let source = "trait Offset where amount:.u8'builtin {
        quote fn add(value:.u8'builtin)->result:.u8'builtin
            where requires value <= 255 - amount, ensures result >= amount;
    }
    struct Add {}
    impl Offset<3> for Add {
        quote fn add(value:.u8'builtin)->.u8'builtin where requires value <= 252 {value + 3}
    }
    fn run()->bool {Add.as(Offset<3>).add(250) == 253}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = source.replace("add(250)", "add(253)");
    assert!(compile(&invalid, Emit::Nir).is_err());
}

#[test]
fn quote_dependent_signed_value_contracts_preserve_negative_bounds() {
    let source = "trait Lower where T,minimum:T, requires T == .i8'builtin {
        quote fn keep(value:T)->result:T where requires value >= minimum, ensures result >= minimum;
    }
    struct Keep {}
    impl Lower<.i8'builtin,-5> for Keep {
        quote fn keep(value:.i8'builtin)->.i8'builtin where requires value >= -5 {value}
    }
    fn run()->bool {Keep.as(Lower<.i8'builtin,-5>).keep(-2) == -2}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = source.replace("keep(-2)", "keep(-6)");
    assert!(compile(&invalid, Emit::Nir).is_err());
}

#[test]
fn quote_value_clauses_expand_without_runtime_call_residue() {
    let source = "struct Flag where enabled:bool {}
        quote fn read(value:Flag<flag>)->bool where flag:bool,.fallback:bool=flag {fallback}
        fn run()->bool {read(Flag<true>{}) and not read(Flag<false>{})
            and read<false,.fallback=true>(Flag<false>{})}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(calls, ["run"]);
}

#[test]
fn quote_trait_value_clauses_expand_selected_and_default_bodies() {
    for implementation in [
        "impl Read<true> for Host {}",
        "impl Read<true> for Host {quote fn read(self,value:Flag<flag>)->bool where flag:bool {flag}}",
    ] {
        let source = format!(
            "struct Flag where enabled:bool {{}}
            trait Read where outer:bool {{
                quote fn read(self,value:Flag<flag>)->bool where flag:bool {{outer and flag}}
            }}
            struct Host {{}}
            {implementation}
            fn run()->bool {{Host{{}}.as(Read<true>).read(Flag<true>{{}})
                and not Host{{}}.as(Read<true>).read<false>(Flag<false>{{}})}}"
        );
        let (value, calls) = execute(&source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true));
        assert_eq!(calls, ["run"]);
    }
}

#[test]
fn quote_implementations_supply_ordinary_trait_callable_entries() {
    for body in [
        "Flag {}.as(Access).read()",
        "invoke(Flag {})",
        "let action = Flag.as(Access).read; action(Flag {})",
    ] {
        let source = format!(
            "trait Access {{fn read(self)->bool;}}
            struct Flag {{}}
            quote fn identity(value:bool)->bool {{value}}
            impl Access for Flag {{quote fn read(self)->bool {{identity(true)}}}}
            fn invoke(value:T)->bool where T:-Access {{value.as(Access).read()}}
            fn run()->bool {{{body}}}"
        );
        let (value, calls) = execute(&source, vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true), "{body}");
        assert!(
            calls.iter().any(|name| name.ends_with(".callable")),
            "{calls:?}"
        );
        assert!(!calls.iter().any(|name| name == "identity"), "{calls:?}");
    }
}

#[test]
fn quote_callable_adapters_preserve_generic_and_value_telescopes() {
    let source = "struct Flag where enabled:bool {}
        trait Read where outer:bool {
            fn read(self,value:Flag<flag>)->bool where flag:bool;
        }
        struct Host where T {}
        impl Read<true> for Host<T> where T {
            quote fn read(self,value:Flag<flag>)->bool where flag:bool {flag}
        }
        fn forward(host:H)->bool where H:-Read<true> {
            host.as(Read<true>).read(Flag<true>{})
        }
        fn run()->bool {forward(Host<bool>{})
            and not Host<.char'builtin>{}.as(Read<true>).read<false>(Flag<false>{})}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn quote_callable_adapters_preserve_public_defaults_and_single_evaluation() {
    let source = "trait Select {fn choose(value:bool,.fallback:bool=value)->bool;}
        struct Host {}
        impl Select for Host {quote fn choose(input:bool,.other:bool=false)->bool {other}}
        fn input()->bool {true}
        fn run()->bool {Host.as(Select).choose(input())}";
    let (value, calls) = execute(source, vec![]);
    assert_eq!(value, comptime_interpretor::Value::Bool(true));
    assert_eq!(
        calls.iter().filter(|name| name.as_str() == "input").count(),
        1
    );
}

#[test]
fn quote_callable_adapters_preserve_interface_contracts() {
    let source = "trait Select where enabled:bool {
        fn choose(value:bool)->result:bool where requires value == enabled, ensures result == enabled;
    }
    struct Switch {}
    impl Select<true> for Switch {quote fn choose(value:bool)->bool {value}}
    impl Select<false> for Switch {quote fn choose(value:bool)->bool {value}}
    fn run()->bool {Switch.as(Select<true>).choose(true) and not Switch.as(Select<false>).choose(false)}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    for invalid in [
        source.replace("choose(true)", "choose(false)"),
        source.replace("->bool {value}", "->bool {not value}"),
    ] {
        let failure = compile(&invalid, Emit::Nir)
            .err()
            .expect("invalid contract accepted");
        assert!(failure.contains("not proven"), "{failure}");
    }
}

#[test]
fn quote_callable_adapters_preserve_staged_inputs_and_effects() {
    let source = "effect poll()->bool;
        trait Select {fn choose(comptime enabled:bool,value:bool)->#poll bool;}
        struct Host {}
        impl Select for Host {
            quote fn choose(comptime enabled:bool,value:bool)->#poll bool {
                poll()#; if enabled {return value;} false
            }
        }
        fn run()->bool {Host.as(Select).choose(true,true)# {poll()=>false,}}";
    assert_eq!(
        execute(&source.replace("quote fn choose", "fn choose"), vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn quote_callable_reference_receivers_preserve_writes_and_public_defaults() {
    let source = "trait Flip {fn flip(*self,.flag:bool=true)->bool;}
        struct Flag {value:bool}
        impl Flip for Flag {quote fn flip(*self,.replacement:bool=false)->bool {
            self.value.* = replacement;self.value.*
        }}
        fn flip(value:*T)->bool where T :- Flip {value.as(Flip).flip()}
        fn run()->bool {var value=Flag {value:false};flip(value.ref) and value.value}";
    assert_eq!(
        execute(source, vec![]).0,
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn quote_callable_cleanup_invalidates_external_observations() {
    let source = "trait Retire {fn drop(*self);}
        struct Watch {target:*bool}
        impl Retire for Watch {quote fn drop(*self) {self.target.*.* = false;}}
        fn change(p:*bool) {let watch=Watch {target:p};}
        fn rejected(p:*bool)->result:bool where ensures result {
            let before=p.*;change(p);before==p.*
        }";
    let failure = compile(source, Emit::Nir)
        .err()
        .expect("cleanup write must invalidate the observation");
    assert!(failure.contains("postcondition"), "{failure}");
}

#[test]
fn quote_callable_consumers_apply_inferred_cleanup_memory_effects() {
    for modifier in ["", "quote "] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            trait Consume {{fn consume(self);}}
            struct Held {{target:*bool}}
            impl Retire for Held {{{modifier}fn drop(*self) {{assumes not self.target.*.*;self.target.*.* = true;}}}}
            impl Consume for Held {{{modifier}fn consume(self) {{}}}}
            fn run()->bool {{var flag=false;Held {{target:flag.ref}}.as(Consume).consume();flag}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap();
        assert_eq!(result, comptime_interpretor::Value::Bool(true));
        // The inferred destructor footprint must not bypass its predicate, nor
        // execute cleanup twice after moving the owner into the consumer.
        let invalid = source.replace("var flag=false", "var flag=true");
        let Artifact::Nir(package) = compile(&invalid, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000,
            )
            .is_err(),
            "cleanup with a false predicate was admitted: {modifier}"
        );
    }
}

#[test]
fn unsafe_quote_calls_are_checked_before_expansion_erases_the_call() {
    for caller in [
        "fn run()->bool {unsafe {quoted(true)}}",
        "unsafe fn run()->bool {quoted(true)}",
    ] {
        let source = format!(
            "unsafe fn leaf(value:bool)->bool {{value}}
            unsafe quote fn quoted(value:bool)->bool {{leaf(value)}} {caller}"
        );
        compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
    for declarations in [
        "unsafe quote fn quoted(value:bool)->bool {value}",
        "unsafe fn leaf(value:bool)->bool {value}
         unsafe quote fn quoted(value:bool)->bool {leaf(value)}",
        "unsafe quote fn leaf(value:bool)->bool {value}
         quote fn quoted(value:bool)->bool {leaf(value)}",
    ] {
        let source = format!("{declarations} fn run()->bool {{quoted(true)}}");
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("unsafe quote call accepted");
        assert!(error.contains("unsafe"), "{error}");
    }
}
