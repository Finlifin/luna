use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[path = "support/execution.rs"]
mod execution_support;
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;

#[test]
fn signature_templates_preserve_closure_environment_types_and_dependencies() {
    for declaration in [
        "template fn maker(value:bool)->%{value:bool} fn()->bool;",
        "typealias Type=.Type'builtin;template fn maker(comptime T:Type,value:T)->%{value:T} fn()->T;",
    ] {
        let argument = if declaration.contains("comptime T") {
            "bool,true"
        } else {
            "true"
        };
        let source = format!(
            "{declaration}
            fn first use maker {{||value}}
            fn second use maker {{||value}}
            template fn invoke(callback:%{{value:bool}} fn()->bool)->bool;
            fn apply use invoke {{callback()}}
            fn run()->bool {{apply(first({argument})) and apply(second({argument}))}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|e| panic!("{declaration}: {e}"))
        else {
            unreachable!()
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    let source = "template fn maker(initial:bool)->%{state:bool} fn()->bool;
        fn first use maker {var state=initial;|| {state=not state;state}}
        fn second use maker {var state=initial;|| {state=not state;state}}
        fn run()->bool {let a=first(false);let b=second(true);a() and not b() and not a() and b()}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let error = compile(
        "template fn maker(value:bool)->%{value:*bool} fn()->bool;fn wrong use maker {||value}",
        Emit::Nir,
    )
    .err()
    .expect("incompatible capture types must be rejected");
    assert!(
        !error.contains("substitution not yet implemented"),
        "{error}"
    );
}

#[test]
fn signature_template_local_destructuring_uses_cleanup_evidence() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for default in [
        "do {let {flag,target}=Cell {flag:input,target:pointer};flag}",
        "do {let cell=Cell {flag:input,target:pointer};let {flag,target}=cell;flag}",
        "Cell {flag:input,target:pointer} match {{flag,target}=>flag}",
    ] {
        for template in [false, true] {
            let declaration = if template {
                format!(
                    "template fn interface(input:bool,pointer:*bool,.value:bool={default})->bool;fn take use interface {{value}}"
                )
            } else {
                format!("fn take(input:bool,pointer:*bool,.value:bool={default})->bool {{value}}")
            };
            let source = format!("trait Copy {{}} trait Drop {{fn drop(*self);}}
                struct Cell {{flag:bool,target:*bool}}
                impl Drop for Cell {{fn drop(*self) {{}}}}
                {declaration}
                fn run()->bool {{var watched=false;let answer=take(true,watched.ref);answer and not watched}}");
            let package = execution_support::compile_with_source_bindings(
                &source,
                &Default::default(),
                selectors,
            )
            .unwrap_or_else(|e| panic!("template={template}, {default}: {e}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
                    .unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            let mutating = source.replace(
                "fn drop(*self) {}",
                "fn drop(*self) {self.target.*.* = true;}",
            );
            let error = execution_support::compile_with_source_bindings(
                &mutating,
                &Default::default(),
                selectors,
            )
            .unwrap_err();
            assert!(
                error.contains("P3 stability/termination"),
                "template={template}, {default}: {error}"
            );
        }
    }
}

#[test]
fn signature_template_defaults_keep_private_locals_and_pattern_bindings() {
    for default in [
        "do {let (left,right)=(input,not input);var local=left;local=not right;local}",
        "input match {value if value=>value,_=>false}",
        "input match {true or false as value=>value}",
        "(|value:bool|value)(input)",
        "do {let action=|value:bool|value;action(input)}",
        "do {let shadow=input;(|value:bool|shadow)(false)}",
        "do {let input=not input;not input}",
        "do {let outer=input;(|a:bool|(|b:bool|outer)(a))(false)}",
    ] {
        // The same source expression must have the ordinary default's behavior.
        for template in [false, true] {
            let declarations = if template {
                format!(
                    "template fn interface(input:bool,.flag:bool={default})->bool;
                    fn first use interface {{flag}} fn second use interface {{flag}}"
                )
            } else {
                format!(
                    "fn first(input:bool,.flag:bool={default})->bool {{flag}}
                    fn second(input:bool,.flag:bool={default})->bool {{flag}}"
                )
            };
            let source = format!(
                "{declarations}
                fn run()->bool {{first(true) and not second(false) and first(false,.flag=true)}}"
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir)
                .unwrap_or_else(|e| panic!("template={template}, {default}: {e}"))
            else {
                unreachable!()
            };
            assert_eq!(
                luna::execution::interpret(&package, "run", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true),
                "template={template}, {default}"
            );
        }
    }
}

#[test]
fn signature_template_default_evaluation_count_and_order_are_preserved() {
    let source = "fn input()->bool {true}
        fn observe(value:bool)->bool {value}
        template fn select(value:bool,.selected:bool=do {let copied=value;observe(copied)})->bool;
        fn first use select {selected}
        fn second use select {selected}
        fn run()->bool {first(input()) and not second(false) and first(false,.selected=true)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(execution_support::id(&package, "run"), vec![])
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert_eq!(
        execution_support::calls(&package, &vm),
        [
            "run", "input", "observe", "first", "observe", "second", "first"
        ]
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn signature_template_constructor_defaults_preserve_fields_and_overrides() {
    let source = "struct Cell {flag:bool}
        fn negate(value:bool)->bool {not value}
        template fn select(input:bool,.cell:Cell=(Cell {flag:negate(input)}))->bool;
        fn first use select {cell.flag}
        fn second use select {cell.flag}
        fn run()->bool {first(false) and not second(true) and second(true,.cell=Cell {flag:true})}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn signature_template_defaults_preserve_admission_and_scope_checks() {
    for (default, expected) in [
        (
            "do {var local=not input;while local != input {local=not input;}local}",
            "P3 stability/termination",
        ),
        ("do {input=false;input}", "P3 stability/termination"),
        // Without bound cleanup protocols this fixture still lacks the evidence
        // supplied by the separate Drop-aware nominal destructuring tests.
        (
            "do {let {flag}=Cell {flag:input};flag}",
            "P3 stability/termination",
        ),
    ] {
        for template in [false, true] {
            let source = if template {
                format!(
                    "template fn interface(input:bool,.flag:bool={default})->bool;fn first use interface {{flag}}"
                )
            } else {
                format!("fn first(input:bool,.flag:bool={default})->bool {{flag}}")
            };
            let source = format!("struct Cell {{flag:bool}} {source}");
            let error = compile(&source, Emit::Nir).err().unwrap_or_else(|| {
                panic!("template={template}, {default}: unsafe default must be rejected")
            });
            assert!(
                error.contains(expected),
                "template={template}, {default}: {error}"
            );
        }
    }
    // An unconditional break provides finite-completion evidence; it must not
    // be conflated with the potentially nonterminating loop above.
    for default in ["do {var local=not input;while local != input {local=input;break;}local}"] {
        for template in [false, true] {
            let declaration = if template {
                format!(
                    "template fn interface(input:bool,.flag:bool={default})->bool;fn first use interface {{flag}}"
                )
            } else {
                format!("fn first(input:bool,.flag:bool={default})->bool {{flag}}")
            };
            let source = format!(
                "struct Cell {{flag:bool}} {declaration}
                fn run()->bool {{first(true) and not first(false) and first(false,.flag=true)}}"
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir)
                .unwrap_or_else(|e| panic!("template={template}, {default}: {e}"))
            else {
                unreachable!()
            };
            assert_eq!(
                luna::execution::interpret(&package, "run", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        }
    }
    assert!(compile("template fn interface(input:bool,.flag:bool=do {let private=input;private})->bool;fn first use interface {private}", Emit::Hir).is_err());
}

#[test]
fn signature_template_instances_execute_with_independent_bindings() {
    let source = "template fn unary(value:bool)->bool;
        fn first use unary {value}
        inline fn second use unary {not value}
        quote fn third use unary {value}
        fn run()->bool {first(true) and second(false) and third(true)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn signature_templates_inherit_defaults_and_contract_bindings() {
    let source = "template fn choose(input:bool,.flag:bool=input)->result:bool
            where ensures result == flag;
        fn first use choose {flag}
        fn second use choose {flag}
        fn run()->bool {first(true) and not second(false) and second(false,.flag=true)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(compile("template fn choose(input:bool)->result:bool where ensures result == input;fn wrong use choose {not input}", Emit::Nir).is_err());
}

#[test]
fn signature_templates_rebind_staged_dependencies_and_default_control_flow() {
    let source = "typealias Type=.Type'builtin;
        template fn identity(comptime T:Type,value:T)->T;
        fn keep use identity {value}
        template quote fn select(flag:bool,.value:bool=if flag {true} else {false})->bool;
        fn first use select {value}
        quote fn second use select {value}
        fn run()->bool {keep(bool,true) and first(true) and not second(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn signature_templates_are_not_callable_and_preserve_declaration_lookup() {
    for body in [
        "fn bad()->bool {signature(true)}",
        "fn bad() {let f=signature;}",
    ] {
        let error = compile(
            &format!("template fn signature(value:bool)->bool;{body}"),
            Emit::Nir,
        )
        .err()
        .unwrap();
        assert!(error.contains("not called as functions"), "{error}");
    }
    let Artifact::Nir(package) = compile_with_modules(
        "use templates.unary;typealias Shared=*bool;fn imported use unary {value} fn run()->bool {imported(true)}",
        &[("templates.fl", "typealias Shared=bool;pub template fn unary(value:Shared)->Shared;")],
        Emit::Nir,
    ).unwrap() else { unreachable!() };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn function_template_instances_link_fresh_inputs_and_preserve_explicit_parameters() {
    let source = "fn first use unary {value}
        quote fn second use unary {value}
        quote fn renamed(other:bool) use unary {other}
        quote fn empty() use zero {true}
        template fn unary(value:bool)->bool;
        template fn zero()->bool;";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("expected HIR")
    };
    assert_eq!(
        dump.matches("(function-template-instance ").count(),
        4,
        "{dump}"
    );
    assert_eq!(
        dump.matches("(function-template-input ").count(),
        3,
        "{dump}"
    );
    assert!(dump.contains("(resolved-local (name \"value\")"), "{dump}");
    assert!(
        dump.contains("(resolved-fn-param (name \"other\")"),
        "{dump}"
    );
    let inputs = dump
        .split("(function-template-input ")
        .skip(1)
        .map(|input| {
            let binding = input
                .split("(binding ")
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap();
            let formal = input
                .split("(formal ")
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap();
            (binding.to_owned(), formal.to_owned())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        inputs
            .iter()
            .map(|input| &input.0)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    assert!(
        inputs
            .iter()
            .all(|input| input.1 == inputs[0].1 && input.0 != input.1)
    );
}

#[test]
fn function_template_instances_link_across_source_modules() {
    let Artifact::Hir(dump) = compile_with_modules(
        "use templates.unary;typealias Shared=*bool;fn imported use unary {value}",
        &[(
            "templates.fl",
            "typealias Shared=bool;pub template fn unary(value:Shared)->Shared;",
        )],
        Emit::Hir,
    )
    .unwrap() else {
        panic!("expected HIR")
    };
    assert!(dump.contains("(function-template-instance "), "{dump}");
    let targets = dump
        .split("(resolved-def (name \"Shared\")")
        .skip(1)
        .map(|part| {
            part.split("(target ")
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        targets.len(),
        1,
        "signature types must retain declaration-side identity: {dump}"
    );
}

#[test]
fn only_quote_template_instances_can_write_explicit_parameters() {
    for modifier in ["", "inline ", "pure "] {
        for params in ["", "value:bool"] {
            let source = format!(
                "template fn signature(value:bool)->bool;
                {modifier}fn instance({params}) use signature {{true}}"
            );
            let error = compile(&source, Emit::Hir)
                .err()
                .expect("ordinary instance must inherit its signature");
            assert!(
                error.contains("explicit function template instance parameters require quote fn"),
                "{error}"
            );
        }
    }
    for modifier in ["", "inline ", "pure ", "quote "] {
        let source = format!(
            "template fn signature(value:bool)->bool;
            {modifier}fn instance use signature {{value}} fn run()->bool {{instance(true)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            unreachable!()
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn function_template_instances_reject_wrong_targets_counts_and_scope_leaks() {
    for source in [
        "fn ordinary(value:bool)->bool {value} fn invalid use ordinary {value}",
        "template fn unary(value:bool)->bool;quote fn invalid() use unary {true}",
        "template fn unary(value:bool)->bool;quote fn invalid(a:bool,b:bool) use unary {a}",
        "template fn unary(value:bool)->bool;quote fn invalid(value:bool,value:bool) use unary {value}",
        "template fn unary(value:bool)->bool;comptime fn invalid use unary {value}",
        "template fn unary(value:bool)->bool;fn first use unary {value} fn outside()->bool {value}",
    ] {
        assert!(compile(source, Emit::Hir).is_err(), "accepted {source}");
    }
}

#[test]
fn function_template_declarations_retain_signature_scope_and_category_in_hir() {
    let source = "typealias Flag=bool;
        template fn signature(value:Flag)->Flag;
        template quote fn quoted(value:Flag)->Flag {value}
        template inline fn inlined(value:Flag)->Flag {value}
        template fn contextual(value:Flag,quote lambda body:<current:Flag |- Flag>)->Flag {
            let current=value;~<body>
        }";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("expected HIR")
    };
    assert_eq!(
        dump.matches("(function-template true)").count(),
        4,
        "{dump}"
    );
    assert!(dump.contains("(fn-declaration "), "{dump}");
    assert!(dump.contains("(quote true)"), "{dump}");
    assert!(dump.contains("(inline true)"), "{dump}");
    assert!(dump.contains("(contextual-expr-type "), "{dump}");
    assert!(
        dump.contains("(resolved-fn-param (name \"value\")"),
        "{dump}"
    );
    let error = compile(source, Emit::Nir)
        .err()
        .expect("template expansion is not yet implemented");
    assert!(error.contains("function template expansion"), "{error}");
}

#[test]
fn function_template_declarations_are_visible_across_modules_and_reject_comptime() {
    let Artifact::Hir(dump) = compile_with_modules(
        "use templates.signature;fn ordinary(value:bool)->bool {value}",
        &[(
            "templates.fl",
            "pub template fn signature(value:bool)->bool;",
        )],
        Emit::Hir,
    )
    .unwrap() else {
        panic!("expected HIR")
    };
    assert!(dump.contains("(function-template true)"), "{dump}");
    for source in [
        "template comptime fn invalid(value:bool)->bool {value}",
        "template pure comptime fn invalid(value:bool)->bool;",
    ] {
        let error = compile(source, Emit::Hir)
            .err()
            .expect("comptime function template must be rejected");
        assert!(
            error.contains("function templates cannot define comptime functions"),
            "{error}"
        );
    }
}

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    compile_with_modules(source, &[], emit)
}

fn compile_with_modules(
    source: &str,
    modules: &[(&str, &str)],
    emit: Emit,
) -> Result<Artifact, String> {
    compile_with_meta_list(source, modules, emit, false)
}

fn compile_with_meta_list(
    source: &str,
    modules: &[(&str, &str)],
    emit: Emit,
    meta_list: bool,
) -> Result<Artifact, String> {
    compile_with_constructors(
        source,
        modules,
        emit,
        meta_list.then_some("input.List"),
        None,
    )
}

fn compile_with_tuple(source: &str, emit: Emit) -> Result<Artifact, String> {
    let source = format!(
        "pure comptime fn TupleTypes<T>->.Type'builtin {{T}}
        pure comptime fn Tuple<...Es:TupleTypes<.Type'builtin>>->.Type'builtin {{bool}}
        {source}"
    );
    compile_with_constructors(
        &source,
        &[],
        emit,
        Some("input.TupleTypes"),
        Some("input.Tuple"),
    )
}

fn compile_with_constructors(
    source: &str,
    modules: &[(&str, &str)],
    emit: Emit,
    meta_list: Option<&str>,
    tuple: Option<&str>,
) -> Result<Artifact, String> {
    let directory = std::env::temp_dir().join(format!(
        "test-template-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("main.fl");
    primitive_bindings::write_fixture(&path, source).unwrap();
    for (name, contents) in modules {
        primitive_bindings::write_fixture(directory.join(name), contents).unwrap();
    }
    let result = frontend::compile(&Options {
        dispatch_fuel: None,
        comptime_intrinsics: Vec::new(),
        input: path.clone(),
        emit,
        output: None,
        build_modes: Default::default(),
        build_targets: Default::default(),
        sysroot: None,
        splice_protocol: None,
        meta_list_type: meta_list.map(str::to_owned),
        tuple_type: tuple.map(str::to_owned),
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        comptime_list_pattern_protocol: None,
        iterator_protocol: None,
        into_iterator_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
    });
    std::fs::remove_dir_all(directory).unwrap();
    result
}

#[test]
fn test_template_definition_preserves_context_and_splice_in_hir() {
    let source = "typealias u32 = .u32'builtin; typealias void = .void'builtin; template test fixture where body: < counter: *u32 |- void > { let value: u32 = 0; let counter = value.ref; ~<body>; }";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        unreachable!()
    };
    assert!(dump.contains("(test-template true)"), "{dump}");
    assert!(dump.contains("(test false)"), "{dump}");
    assert!(dump.contains("counter"), "{dump}");
    assert!(dump.contains("splice"), "{dump}");
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert!(package.tests.is_empty());
}

#[test]
fn template_instances_execute_the_composed_body() {
    let Artifact::Nir(package) = compile("typealias V = .void'builtin; template test fixture where body: < |- V > { ~<body>; } test example use fixture { assumes true; }", Emit::Nir).unwrap() else { unreachable!() };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 1);
    assert!(report.is_success(), "{report}");
}

#[test]
fn template_body_routing_uses_role_and_keeps_optional_inputs() {
    let source = "typealias Void = .void'builtin;
        template test fixture where .enabled: bool = true, action: < |- Void >
        { if enabled { ~<action>; } }";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        unreachable!()
    };
    assert!(dump.contains("template-body-binding"), "{dump}");
    assert!(dump.contains("(template-body-binding (some "), "{dump}");
    assert!(dump.contains("enabled"), "{dump}");
    assert!(dump.contains("action"), "{dump}");
}

#[test]
fn test_templates_require_one_anchored_body_input_including_shorthand() {
    for (clauses, message) in [
        ("", "requires exactly one anchored body parameter"),
        (
            "where .enabled: bool = true",
            "requires exactly one anchored body parameter",
        ),
        (
            "where first: < |- bool >, second: < |- bool >",
            "more than one anchored parameter",
        ),
        (
            "where T, body: < |- bool >",
            "more than one anchored parameter",
        ),
        (
            "where quote body: < |- bool >, other: bool",
            "more than one anchored parameter",
        ),
    ] {
        let source = format!("template test invalid {clauses} {{}}");
        let failure = compile(&source, Emit::Hir)
            .err()
            .expect("invalid body routing");
        assert!(failure.contains(message), "{source}: {failure}");
    }
}

#[test]
fn forward_template_instances_bind_only_exported_context_with_fresh_identities() {
    let source = "typealias V = .void'builtin;
        fn flag() -> bool { false }
        test first use fixture { assumes flag; }
        test second use fixture { assumes flag; }
        template test fixture where body: < flag: bool |- V >
        { let secret = false; let flag = true; ~<body>; }";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        dump.matches("(test-template-instance ").count(),
        2,
        "{dump}"
    );
    let ports = dump
        .split("(test-template-port ")
        .skip(1)
        .map(|s| s.split("(body ").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        dump.matches("(test-template-interface ").count(),
        2,
        "{dump}"
    );
    assert_eq!(dump.matches("(formal (some ").count(), 2, "{dump}");
    assert!(!dump.contains("(interface (none))"), "{dump}");
    assert_eq!(ports.len(), 2);
    assert_ne!(
        ports[0], ports[1],
        "instances must have distinct binding identities"
    );
    assert_eq!(
        dump.matches("(resolved-local (name \"flag\")").count(),
        3,
        "{dump}"
    );
    let bad = source.replace("assumes flag;", "assumes secret;");
    let error = compile(&bad, Emit::Hir)
        .err()
        .expect("private local must remain hidden");
    assert!(error.contains("Unresolved identifier `secret`"), "{error}");
}

#[test]
fn qualified_and_imported_templates_supply_context_before_body_lowering() {
    let source = "typealias V = .void'builtin;
        test earlier use fixtures.fixture { assumes exposed; }
        mod fixtures { pub template test fixture where body: < exposed: bool |- V >
            { let exposed = true; ~<body>; } }
        use fixtures.fixture;
        test imported use fixture { assumes exposed; }";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        dump.matches("(test-template-instance ").count(),
        2,
        "{dump}"
    );
}

#[test]
fn template_context_is_available_from_a_separate_source_module() {
    let Artifact::Hir(dump) = compile_with_modules(
        "use fixtures.fixture; test imported use fixture { assumes exposed; }",
        &[("fixtures.fl", "typealias V = .void'builtin; pub template test fixture where body: < exposed: bool |- V > { let exposed = true; ~<body>; }")],
        Emit::Hir,
    ).unwrap() else { unreachable!() };
    assert!(dump.contains("test-template-instance"), "{dump}");
    assert!(dump.contains("test-template-port"), "{dump}");
}

#[test]
fn duplicate_template_ports_and_non_template_targets_are_rejected() {
    for (source, message) in [
        (
            "typealias V = .void'builtin; template test fixture where body: < flag: bool, flag: bool |- V > { ~<body>; } test use_it use fixture {}",
            "duplicate test template context binding",
        ),
        (
            "fn fixture() {} test use_it use fixture {}",
            "requires a declared contextual body interface",
        ),
    ] {
        let error = compile(source, Emit::Hir)
            .err()
            .expect("invalid template interface");
        assert!(error.contains(message), "{error}");
    }
}

#[test]
fn linked_template_types_retain_the_declarations_lexical_scope() {
    let source = "typealias V = .void'builtin;
        mod caller { typealias Shared = *bool;
            test uses_fixture use fixtures.fixture { let saved = exposed; } }
        mod fixtures { typealias Shared = bool;
            pub template test fixture where body: < exposed: Shared |- V >
            { let exposed = true; ~<body>; } }";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        unreachable!()
    };
    let references = dump
        .split("(resolved-def (name \"Shared\")")
        .skip(1)
        .map(|rest| {
            rest.split("(target ")
                .nth(1)
                .unwrap()
                .split(")))")
                .next()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        references.len(),
        2,
        "declaration and linked interface: {dump}"
    );
    assert_eq!(
        references[0], references[1],
        "caller alias must not rebind the interface: {dump}"
    );
    assert!(dump.contains("test-template-interface"), "{dump}");
}

#[test]
fn template_splice_defaults_resolve_local_identities_and_keep_explicit_overrides() {
    let prefix = "typealias V = .void'builtin; template test fixture where body: < first: bool, second: bool |- V >";
    let Artifact::Hir(implicit) = compile(
        &format!("{prefix} {{ let first = true; let second = false; ~<body>; }}"),
        Emit::Hir,
    )
    .unwrap() else {
        unreachable!()
    };
    assert!(
        implicit.contains("(substitute (port (ident (name \"first\")"),
        "{implicit}"
    );
    assert!(
        implicit.contains("(substitute (port (ident (name \"second\")"),
        "{implicit}"
    );
    let Artifact::Hir(explicit) = compile(
        &format!(
            "{prefix} {{ let replacement = true; let second = false; ~[first=replacement]<body>; }}"
        ),
        Emit::Hir,
    )
    .unwrap() else {
        unreachable!()
    };
    assert_eq!(
        explicit
            .matches("(substitute (port (ident (name \"first\")")
            .count(),
        1,
        "{explicit}"
    );
    assert!(
        explicit.contains("(resolved-local (name \"replacement\")"),
        "{explicit}"
    );
    assert!(
        explicit.contains("(substitute (port (ident (name \"second\")"),
        "{explicit}"
    );
}

#[test]
fn template_implicit_substitution_requires_an_available_local() {
    for body in ["~<body>;", "~<body>; let exposed = true;"] {
        let source = format!(
            "typealias V = .void'builtin; fn exposed() -> bool {{ true }} template test fixture where body: < exposed: bool |- V > {{ {body} }}"
        );
        let error = compile(&source, Emit::Hir)
            .err()
            .expect("a global or later declaration is not a context binding");
        assert!(
            error.contains("no local binding for context port `exposed`"),
            "{error}"
        );
    }
}

#[test]
fn template_explicit_substitutions_validate_ports_and_binding_category() {
    for (splice, message) in [
        (
            "~[flag=actual,flag=actual]<body>",
            "duplicate test template splice port",
        ),
        (
            "~[missing=actual]<body>",
            "unknown test template splice port",
        ),
        (
            "~[flag=true]<body>",
            "Code substitution requires a binding identifier",
        ),
        ("~[flag=global]<body>", "requires a local binding"),
        ("~[flag=global()]<body>", "parsing failed"),
    ] {
        let source = format!(
            "typealias V = .void'builtin; fn global() -> bool {{ true }} template test fixture where body: < flag: bool |- V > {{ let flag = false; let actual = true; {splice}; }}"
        );
        let failure = compile(&source, Emit::Hir)
            .err()
            .expect("invalid substitution must be rejected");
        assert!(failure.contains(message), "{splice}: {failure}");
    }
}

#[test]
fn duplicate_template_context_names_are_rejected_without_an_instance() {
    let error = compile("typealias V = .void'builtin; template test fixture where body: < flag: bool, flag: bool |- V > { let flag = true; ~<body>; }", Emit::Hir).err().expect("duplicate interface");
    assert!(
        error.contains("duplicate test template context binding"),
        "{error}"
    );
}

#[test]
fn template_port_resolution_obeys_nested_lexical_scope() {
    let source = "typealias V = .void'builtin; template test fixture where body: < flag: bool |- V > { let flag = false; { let flag = true; ~<body>; } }";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        unreachable!()
    };
    let substitution = dump.split("(substitute (port ").nth(1).unwrap();
    let target = substitution
        .split("(target ")
        .nth(1)
        .unwrap()
        .split(")))")
        .next()
        .unwrap();
    let bindings = dump
        .split("(pattern (id ")
        .skip(1)
        .map(|part| part.split(")))").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(bindings.len(), 2, "{dump}");
    assert_ne!(bindings[0], bindings[1]);
    assert_eq!(
        target, bindings[1],
        "splice must select the inner binding: {dump}"
    );
    let error = compile("typealias V = .void'builtin; template test fixture where body: < flag: bool |- V > { { let flag = true; } ~<body>; }", Emit::Hir).err().expect("block-local must not escape");
    assert!(
        error.contains("no local binding for context port `flag`"),
        "{error}"
    );
}

#[test]
fn template_instances_run_fresh_setup_and_retain_post_splice_observations() {
    let source = "typealias V = .void'builtin;
        template test fixture where body: < state: *bool |- V > {
            var value = false; let state = value.ref; ~<body>; assumes value;
        }
        test first use fixture { assumes not state.*; state.* = true; }
        test ordinary { assumes true; }
        test second use fixture { assumes not state.*; state.* = true; }
        test fails use fixture { assumes not state.*; }
        test last use fixture { state.* = true; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(
        report
            .results
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "ordinary", "second", "fails", "last"]
    );
    assert_eq!(report.failures(), 1, "{report}");
    assert!(report.results[3].failure.is_some(), "{report}");
}

#[test]
fn template_bodies_and_substitutions_are_type_checked_even_without_instances() {
    for source in [
        "typealias V = .void'builtin; template test fixture where body: < flag: bool |- V > { let flag = (true,false); ~<body>; }",
        "typealias V = .void'builtin; template test fixture where body: < |- V > { let wrong: bool = (true,false); ~<body>; }",
        "typealias T = .Type'builtin; typealias V = .void'builtin; template test fixture where body: < flag: T |- V > { let flag = bool; ~<body>; }",
    ] {
        assert!(compile(source, Emit::Nir).is_err(), "accepted {source}");
    }
    let error = compile("typealias V = .void'builtin; template test fixture where body: < flag: bool |- V > { let flag = true; ~<body>; } test bad use fixture { let wrong: *bool = flag; }", Emit::Nir).err().expect("instance port type must be enforced");
    assert!(error.contains("type mismatch"), "{error}");
}

#[test]
fn template_setup_resources_are_cleaned_by_the_checked_test_runner() {
    let source = "typealias V = .void'builtin;
        trait Duplicate {} trait Retire { fn drop(*self); }
        struct Ticket { closed: *bool }
        impl Retire for Ticket { fn drop(*self) { self.closed.*.* = true; } }
        template test fixture where body: < closed: *bool |- V > {
            var done = false; let closed = done.ref;
            { let ticket = Ticket { closed: closed }; ~<body>; }
            assumes done;
        }
        test first use fixture { assumes not closed.*; }
        test second use fixture { assumes not closed.*; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    for inline in [false, true] {
        let report =
            luna::testing::run_report(&package, 0, 100_000, inline, Some(&protocols)).unwrap();
        assert_eq!(report.results.len(), 2);
        assert!(report.is_success(), "{report}");
    }
}

#[test]
fn template_body_result_follows_its_contextual_interface() {
    let Artifact::Nir(package) = compile("template test check_boolean where body: < |- bool > { assumes ~<body>; } test succeeds use check_boolean { true } test fails use check_boolean { false }", Emit::Nir).unwrap() else { unreachable!() };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 2);
    assert!(report.results[0].failure.is_none(), "{report}");
    assert!(report.results[1].failure.is_some(), "{report}");
}

#[test]
fn executable_template_substitutions_preserve_explicit_port_correspondence() {
    let source = "typealias V = .void'builtin;
        template test fixture where body: < first: bool, second: bool |- V > {
            let a = true; let b = false; ~[first=b,second=a]<body>;
        }
        test swapped use fixture { assumes not first and second; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 1);
    assert!(report.is_success(), "{report}");
}

#[test]
fn templates_in_source_modules_execute_in_the_primary_test_suite() {
    let Artifact::Nir(package) = compile_with_modules(
        "use fixtures.fixture; test imported use fixture { assumes exposed; }",
        &[("fixtures.fl", "typealias V = .void'builtin; pub template test fixture where body: < exposed: bool |- V > { let exposed = true; ~<body>; }")],
        Emit::Nir,
    ).unwrap() else { unreachable!() };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 1);
    assert!(report.is_success(), "{report}");
}

#[test]
fn template_optional_defaults_compute_in_declaration_order() {
    let source = "typealias V = .void'builtin;
        comptime fn invert(value:bool)->bool { not value }
        template test fixture where body:< flag:bool |- V >,
            .initial:bool=false,.enabled:bool=invert(initial) {
            let flag=enabled; ~<body>;
        }
        test first use fixture { assumes flag; }
        test second use fixture { assumes flag; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 2);
    assert!(report.is_success(), "{report}");
}

#[test]
fn template_optional_defaults_reject_bad_types_and_forward_dependencies() {
    for source in [
        "typealias V=.void'builtin; template test fixture where body:<|-V>,.flag:bool='x' { ~<body>; }",
        "typealias V=.void'builtin; template test fixture where body:<|-V>,.flag:bool=flag { ~<body>; }",
        "typealias V=.void'builtin; template test fixture where body:<|-V>,.first:bool=second,.second:bool=true { ~<body>; }",
        "typealias V=.void'builtin; template test fixture where body:<|-V>,.flag:bool=true { let wrong:*bool=flag; ~<body>; }",
    ] {
        assert!(compile(source, Emit::Nir).is_err(), "accepted {source}");
    }
}

#[test]
fn template_type_defaults_preserve_aliases_and_reject_non_types() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V>,.T:.Type'builtin=bool,.U:.Type'builtin=T {
            let same:bool=T == U; assumes same; ~<body>;
        }
        test first use fixture {}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert!(report.is_success(), "{report}");
    assert!(compile("typealias V=.void'builtin; template test fixture where body:<|-V>,.T:.Type'builtin=true { ~<body>; }", Emit::Nir).is_err());
}

#[test]
fn unused_template_defaults_are_checked_without_invocation() {
    let definition = "typealias V=.void'builtin;
        comptime fn fail_default()->bool { assumes false; true }
        template test fixture where body:<|-V>,.flag:bool=fail_default() {
            let observed=flag; ~<body>;
        }";
    compile(definition, Emit::Nir).expect("unused default must not execute");
    let error = compile(
        &format!("{definition} test selected use fixture {{}}"),
        Emit::Nir,
    )
    .err()
    .expect("selected default must execute at comptime");
    assert!(!error.contains("not yet implemented"), "{error}");
}

#[test]
fn explicit_template_inputs_override_defaults_and_feed_later_defaults() {
    let source = "typealias V=.void'builtin;
        comptime fn invert(value:bool)->bool { not value }
        template test fixture where body:< flag:bool |- V >,
            .initial:bool=false,.enabled:bool=invert(initial) {
            let flag=enabled; ~<body>;
        }
        test defaulted use fixture { assumes flag; }
        test overridden use fixture<.initial=true> { assumes not flag; }
        test both use fixture<.enabled=true,.initial=true> { assumes flag; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 3);
    assert!(report.is_success(), "{report}");
    for args in [
        ".unknown=true",
        ".body=true",
        ".initial=true,.initial=false",
        ".initial='x'",
        "true",
    ] {
        assert!(
            compile(
                &format!("{source} test bad use fixture<{args}> {{}}"),
                Emit::Nir
            )
            .is_err(),
            "accepted {args}"
        );
    }
}

#[test]
fn explicit_template_input_does_not_invoke_replaced_default() {
    let source = "typealias V=.void'builtin;
        comptime fn fail_default()->bool { assumes false; true }
        template test fixture where body:<|-V>,.flag:bool=fail_default() {
            assumes flag; ~<body>;
        }
        test selected use fixture<.flag=true> {}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert!(report.is_success(), "{report}");
}

#[test]
fn template_type_inputs_specialize_the_contextual_body_result() {
    let source = "template test fixture where body:<|-T>,.T:.Type'builtin=bool {
            let result:T=~<body>;
        }
        test boolean use fixture { true }
        test integer use fixture<.T=.u8'builtin> { 7 }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 2);
    assert!(report.is_success(), "{report}");
    let error = compile(
        &format!("{source} test bad use fixture<.T=bool> {{ 'x' }}"),
        Emit::Nir,
    )
    .err()
    .expect("body must use the instantiated interface");
    assert!(error.contains("type mismatch"), "{error}");
}

#[test]
fn imported_template_explicit_inputs_use_call_site_definitions() {
    let Artifact::Nir(package) = compile_with_modules(
        "use fixtures.fixture; const FLAG=false;
         test explicit use fixture<.flag=FLAG> { assumes not exposed; }
         test defaulted use fixture { assumes exposed; }",
        &[(
            "fixtures.fl",
            "typealias V=.void'builtin; const FLAG=true;
            pub template test fixture where body:< exposed:bool |- V >,.flag:bool=FLAG {
                let exposed=flag; ~<body>;
            }",
        )],
        Emit::Nir,
    )
    .unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 2);
    assert!(report.is_success(), "{report}");
}

#[test]
fn template_program_contracts_are_retained_on_generated_tests() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V>,.entry:bool=true,.exit:bool=true,
            requires entry,ensures exit { ~<body>; }
        test succeeds use fixture {}
        test bad_entry use fixture<.entry=false> {}
        test early_exit use fixture { return; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    for function in package
        .functions
        .iter()
        .filter(|function| package.tests.contains(&function.declaration))
    {
        assert_eq!(function.preconditions.len(), 1);
        assert_eq!(function.postconditions.len(), 1);
    }
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 3);
    assert!(report.results[0].failure.is_none(), "{report}");
    assert!(report.results[1].failure.is_some(), "{report}");
    assert!(report.results[2].failure.is_none(), "{report}");
    for body in ["", "return;"] {
        let error = compile(
            &format!("{source} test bad_exit use fixture<.exit=false> {{ {body} }}"),
            Emit::Nir,
        )
        .err()
        .expect("postcondition must hold on every exit");
        assert!(error.contains("postcondition is not proven"), "{error}");
    }
}

#[test]
fn template_type_requirements_are_checked_for_actual_inputs() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V>,.T:.Type'builtin=bool,
            requires T == bool { ~<body>; }
        test succeeds use fixture {}";
    compile(source, Emit::Nir).unwrap();
    let error = compile(
        &format!("{source} test fails use fixture<.T=.u8'builtin> {{}}"),
        Emit::Nir,
    )
    .err()
    .expect("failed type prerequisite must prevent instantiation");
    assert!(
        error.contains("type requirement is not satisfied"),
        "{error}"
    );
}

#[test]
fn template_contracts_check_unused_definitions_and_value_equalities() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V>,.a:bool=true,.b:bool=true,
            requires a == b { ~<body>; }
        test succeeds use fixture {}
        test fails use fixture<.b=false> {}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert!(report.results[0].failure.is_none(), "{report}");
    assert!(report.results[1].failure.is_some(), "{report}");
    for clause in ["requires 'x'", "ensures 'x'", "decreases true"] {
        assert!(compile(&format!("typealias V=.void'builtin; template test fixture where body:<|-V>,{clause} {{ ~<body>; }}"), Emit::Nir).is_err(), "accepted {clause}");
    }
}

#[test]
fn template_return_skips_setup_tail_without_capturing_loop_targets() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V> { ~<body>; assumes false; }
        test early use fixture { return; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert!(report.is_success(), "{report}");
    for body in ["return true;", "break;", "continue;"] {
        assert!(
            compile(
                &format!("{source} test bad use fixture {{ {body} }}"),
                Emit::Nir
            )
            .is_err(),
            "accepted {body}"
        );
    }
}

#[test]
fn template_early_return_runs_cleanup_in_checked_and_inline_modes() {
    let source = "typealias V=.void'builtin;
        trait Duplicate {} trait Retire { fn drop(*self); }
        struct Ticket { ready:bool }
        impl Retire for Ticket { fn drop(*self) { assumes self.ready.*; } }
        template test fixture where body:< ticket:*Ticket |- V > {
            var value=Ticket {ready:false}; let ticket=value.ref; ~<body>;
            assumes false;
        }
        test ready use fixture { ticket.ready.* = true; return; }
        test unready use fixture { return; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    for inline in [false, true] {
        let report =
            luna::testing::run_report(&package, 0, 100_000, inline, Some(&protocols)).unwrap();
        assert!(report.results[0].failure.is_none(), "{report}");
        assert!(report.results[1].failure.is_some(), "{report}");
    }
}

#[test]
fn ordinary_quotes_inside_template_bodies_do_not_inherit_test_returns() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V> { ~<body>; }
        test bad use fixture { const code='(do { return; }); }";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("nested quote must own its control boundary");
    assert!(error.contains("return outside a function"), "{error}");
}

#[test]
fn template_trait_prerequisites_use_the_checked_implementation_world() {
    let source = "typealias V=.void'builtin;
        trait Read { fn read()->bool; } trait Tag {}
        impl Read for bool { fn read()->bool { true } }
        impl Tag for bool {}
        template test fixture where body:< flag:bool |- V >,.T:.Type'builtin=bool,
            requires T :- Read + Tag {
            let flag=T.as(Read).read(); ~<body>;
        }
        test selected use fixture { assumes flag; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert!(report.is_success(), "{report}");
    let error = compile(
        &source.replace("use fixture {", "use fixture<.T=.char'builtin> {"),
        Emit::Nir,
    )
    .err()
    .expect("missing implementation must reject instantiation");
    assert!(error.contains("type requirement"), "{error}");
    assert!(compile(&source.replace("impl Tag for bool {}", ""), Emit::Nir).is_err());
}

#[test]
fn template_trait_disjunction_does_not_supply_a_single_branch_method() {
    let source = "typealias V=.void'builtin;
        trait Read { fn read()->bool; } trait Tag {}
        impl Read for bool { fn read()->bool { true } }
        template test fixture where body:<|-V>,.T:.Type'builtin=bool,
            requires (T :- Read) or (T :- Tag) {
            let value=T.as(Read).read(); ~<body>;
        }
        test selected use fixture {}";
    assert!(
        compile(source, Emit::Nir).is_err(),
        "disjunction cannot authorize Read in the generic body"
    );
    let source = source.replace("let value=T.as(Read).read();", "");
    let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert!(
        luna::testing::run_report(&package, 0, 100_000, false, None)
            .unwrap()
            .is_success()
    );
}

#[test]
fn template_trait_requirements_resolve_associated_type_prerequisites() {
    let source = "typealias V=.void'builtin;
        trait Source { assoc Item:.Type'builtin; } trait Allowed {}
        impl Source for bool { assoc Item:.Type'builtin=.char'builtin; }
        impl Allowed for .char'builtin {}
        template test fixture where body:<|-V>,.T:.Type'builtin=bool,
            requires T :- Source, requires T.as(Source).Item :- Allowed { ~<body>; }
        test selected use fixture {}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert!(
        luna::testing::run_report(&package, 0, 100_000, false, None)
            .unwrap()
            .is_success()
    );
    let error = compile(
        &source.replace("impl Allowed for .char'builtin {}", ""),
        Emit::Nir,
    )
    .err()
    .expect("associated prerequisite must be proved independently");
    assert!(error.contains("type requirement"), "{error}");
}

#[test]
fn template_trait_prerequisites_are_prepared_independently_of_clause_order() {
    let declarations = "typealias V=.void'builtin;
        trait Source { assoc Item:.Type'builtin; } trait Allowed {}
        impl Source for bool { assoc Item:.Type'builtin=.char'builtin; }
        impl Allowed for .char'builtin {}";
    for constraints in [
        "requires T.as(Source).Item :- Allowed, requires T :- Source",
        "requires (T.as(Source).Item :- Allowed) and (T :- Source)",
    ] {
        let source = format!("{declarations}
            template test fixture where body:<|-V>,.T:.Type'builtin=bool,{constraints} {{ ~<body>; }}
            test first use fixture {{}} test second use fixture {{}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            unreachable!()
        };
        let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
        assert_eq!(report.results.len(), 2);
        assert!(report.is_success(), "{report}");
        assert!(
            compile(
                &source.replace("impl Allowed for .char'builtin {}", ""),
                Emit::Nir
            )
            .is_err()
        );
    }
}

#[test]
fn template_associated_prerequisites_do_not_invent_missing_trait_evidence() {
    let declarations = "typealias V=.void'builtin;
        trait Source { assoc Item:.Type'builtin; } trait Marker {} trait Allowed {}
        impl Source for bool { assoc Item:.Type'builtin=.char'builtin; }
        impl Allowed for .char'builtin {}";
    for constraints in [
        "requires T.as(Source).Item :- Allowed",
        "requires T.as(Source).Item :- Allowed, requires (T :- Source) or (T :- Marker)",
    ] {
        let source = format!("{declarations}
            template test fixture where body:<|-V>,.T:.Type'builtin=bool,{constraints} {{ ~<body>; }}");
        let failure = compile(&source, Emit::Nir)
            .err()
            .expect("abstract definition needs positive Source evidence");
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
    let source = format!(
        "{declarations}
        impl Source for .char'builtin {{ assoc Item:.Type'builtin=bool; }}
        template test fixture where body:<|-V>,.T:.Type'builtin=bool,
            requires T.as(Source).Item :- Allowed, requires T :- Source {{ ~<body>; }}
        test first use fixture {{}} test second use fixture {{}}
        test rejected use fixture<.T=.char'builtin> {{}}"
    );
    let failure = compile(&source, Emit::Nir)
        .err()
        .expect("prior instance evidence must not satisfy a different associated type");
    assert!(failure.contains("type requirement"), "{failure}");
}

#[test]
fn template_type_defaults_use_trait_evidence_from_completed_inputs() {
    let source = "trait Source { assoc Item:.Type'builtin; }
        impl Source for bool { assoc Item:.Type'builtin=.char'builtin; }
        impl Source for .char'builtin { assoc Item:.Type'builtin=bool; }
        template test fixture where body:<|-U>,.T:.Type'builtin=bool,
            .U:.Type'builtin=T.as(Source).Item,requires T :- Source {
            let value:U=~<body>;
        }
        test character use fixture { 'x' }
        test boolean use fixture<.T=.char'builtin> { true }
        test explicit use fixture<.U=bool> { false }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 3);
    assert!(report.is_success(), "{report}");
    let error = compile(
        &source.replace(
            "test character use fixture { 'x' }",
            "test character use fixture { true }",
        ),
        Emit::Nir,
    )
    .err()
    .expect("associated default must specialize the body interface");
    assert!(error.contains("type mismatch"), "{error}");
}

#[test]
fn template_default_evidence_cannot_read_later_inputs() {
    let source = "trait Source { assoc Item:.Type'builtin; }
        impl Source for bool { assoc Item:.Type'builtin=bool; }
        template test fixture where body:<|-U>,.U:.Type'builtin=T.as(Source).Item,
            .T:.Type'builtin=bool,requires T :- Source { let value:U=~<body>; }";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("later input must not be made available by its trait bound");
    assert!(error.contains("later parameter"), "{error}");
}

#[test]
fn template_value_defaults_can_invoke_constrained_comptime_methods() {
    let source = "typealias V=.void'builtin;
        trait Config { comptime fn enabled()->bool; }
        impl Config for bool { comptime fn enabled()->bool { true } }
        impl Config for .char'builtin { comptime fn enabled()->bool { assumes false; false } }
        template test fixture where body:< flag:bool |- V >,.T:.Type'builtin=bool,
            .flag:bool=T.as(Config).enabled(),requires T :- Config {
            let flag=flag; ~<body>;
        }
        test computed use fixture { assumes flag; }
        test replaced use fixture<.T=.char'builtin,.flag=false> { assumes not flag; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 2);
    assert!(report.is_success(), "{report}");
    assert!(
        compile(
            &source.replace(".T=.char'builtin,.flag=false", ".T=.char'builtin"),
            Emit::Nir
        )
        .is_err()
    );
    assert!(
        compile(&source.replace(",requires T :- Config", ""), Emit::Nir).is_err(),
        "default needs abstract Config evidence even when concrete defaults happen to implement it"
    );
}

#[test]
fn template_postconditions_can_observe_old_immutable_clause_values() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V>,.flag:bool=true,
            ensures flag == flag'old { ~<body>; }
        test first use fixture {}
        test second use fixture<.flag=false> { return; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    for function in package
        .functions
        .iter()
        .filter(|function| package.tests.contains(&function.declaration))
    {
        assert_eq!(function.postconditions.len(), 1);
        assert!(
            function.entry_snapshots.is_empty(),
            "comptime immutable inputs do not need runtime snapshots"
        );
    }
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 2);
    assert!(report.is_success(), "{report}");
    let error = compile(
        &source.replace("ensures flag == flag'old", "ensures flag'old"),
        Emit::Nir,
    )
    .err()
    .expect("false old-value postcondition must be rejected");
    assert!(error.contains("postcondition is not proven"), "{error}");
}

#[test]
fn template_old_views_are_scoped_to_postconditions() {
    for source in [
        "typealias V=.void'builtin; template test fixture where body:<|-V>,.flag:bool=true,requires flag'old { ~<body>; }",
        "typealias V=.void'builtin; template test fixture where body:<|-V>,.flag:bool=true,ensures flag == flag'old { assumes flag'old; ~<body>; }",
        "typealias V=.void'builtin; template test fixture where body:<|-V>,.flag:bool=true,ensures flag == flag'old { ~<body>; } test bad use fixture { assumes true'old; }",
    ] {
        let error = compile(source, Emit::Nir)
            .err()
            .expect("old scope must not escape its postcondition");
        assert!(
            error.contains("old is only available in program postconditions"),
            "{error}"
        );
    }
}

#[test]
fn template_returns_are_not_retargeted_to_nested_splice_functions() {
    for setup in [
        "let callback=|| { ~<body>; }; callback();",
        "const delayed='(do { ~<body>; }); ~<delayed>;",
    ] {
        let source = format!(
            "typealias V=.void'builtin;
            template test fixture where body:<|-V> {{ {setup} }}
            test early use fixture {{ return; }}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("nonlocal return needs an explicit control transfer implementation");
        assert!(error.contains("code return cannot cross"), "{error}");
    }
}

#[test]
fn template_splices_inside_closures_preserve_local_returns() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V> {
            let callback=|| { ~<body>; }; callback();
        }
        test ordinary use fixture { assumes true; }
        test local_return use fixture { let inner=|| { return true; }; let answer=inner(); assumes answer; }";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 2);
    assert!(report.is_success(), "{report}");
}

#[test]
fn template_tuple_variadic_inputs_collect_with_element_context() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:< flag:bool |- V >,
            ...settings:Tuple<.u8'builtin,bool>,.invert:bool=false {
            let (number,enabled)=settings;
            assumes number == 7; let flag=enabled != invert; ~<body>;
        }
        test positional use fixture<7,true> { assumes flag; }
        test spread use fixture<...(7,false),.invert=true> { assumes flag; }
        test named use fixture<.settings=(7,true)> { assumes flag; }";
    let Artifact::Nir(package) = compile_with_tuple(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 3);
    assert!(report.is_success(), "{report}");
    for args in ["7", "7,true,false", "'x',true", "7,true,.settings=(7,true)"] {
        assert!(
            compile_with_tuple(
                &format!("{source} test bad use fixture<{args}> {{}}"),
                Emit::Nir
            )
            .is_err(),
            "accepted {args}"
        );
    }
}

#[test]
fn template_variadic_collections_feed_defaults_and_follow_type_inputs() {
    let source = "typealias V=.void'builtin;
        comptime fn packet()->Tuple<bool,bool> { (false,true) }
        template test fixture where body:< flag:bool |- V >,.T:.Type'builtin=bool,
            ...values:Tuple<T,bool>,.flag:bool=do { let (_,flag)=values; flag } {
            let flag=flag; ~<body>;
        }
        test booleans use fixture<false,true> { assumes flag; }
        test bytes use fixture<7,false,.T=.u8'builtin> { assumes not flag; }
        test computed_packet use fixture<.values=packet()> { assumes flag; }";
    let Artifact::Nir(package) = compile_with_tuple(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 3);
    assert!(report.is_success(), "{report}");
    assert!(
        compile_with_tuple(
            &source.replace("<7,false,.T=.u8'builtin>", "<256,false,.T=.u8'builtin>"),
            Emit::Nir
        )
        .is_err()
    );
}

#[test]
fn explicit_template_comptime_calls_run_before_selected_contracts() {
    let source = "typealias V=.void'builtin;
        comptime fn choose()->bool { true }
        template test fixture where body:<|-V>,.flag:bool=false,requires flag { ~<body>; }
        test selected use fixture<.flag=choose()> {}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert!(
        luna::testing::run_report(&package, 0, 100_000, false, None)
            .unwrap()
            .is_success()
    );
    assert!(
        compile(
            &source.replace("comptime fn choose", "fn choose"),
            Emit::Nir
        )
        .is_err(),
        "runtime producer cannot supply a comptime input"
    );
}

#[test]
fn computed_template_tuple_spreads_interleave_with_contextual_inputs() {
    let source = "typealias V=.void'builtin;
        comptime fn flags()->Tuple<bool,bool> { (true,false) }
        template test fixture where body:< result:bool |- V >,
            ...values:Tuple<.u8'builtin,bool,bool,.u8'builtin> {
            let (first,a,b,last)=values; let result=first == 7 and a and not b and last == 9;
            ~<body>;
        }
        test selected use fixture<7,...flags(),9> { assumes result; }";
    let Artifact::Nir(package) = compile_with_tuple(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert!(
        luna::testing::run_report(&package, 0, 100_000, false, None)
            .unwrap()
            .is_success()
    );
    for args in [
        "7,...flags()",
        "7,...flags(),9,false",
        "7,...true,9",
        "7,...(do { ('x',false) }),9",
    ] {
        assert!(
            compile_with_tuple(&source.replace("7,...flags(),9", args), Emit::Nir).is_err(),
            "accepted {args}"
        );
    }
    assert!(
        compile_with_tuple(&source.replace("comptime fn flags", "fn flags"), Emit::Nir).is_err()
    );
}

#[test]
fn computed_template_spread_elements_keep_their_completed_types() {
    let source = "typealias V=.void'builtin;
        comptime fn values()->Tuple<.u16'builtin,bool> { (7,true) }
        template test fixture where body:<|-V>,...items:Tuple<.u8'builtin,bool> { ~<body>; }
        test literal use fixture<...(7,true)> {}";
    compile_with_tuple(source, Emit::Nir).unwrap();
    let error = compile_with_tuple(
        &format!("{source} test computed use fixture<...values()> {{}}"),
        Emit::Nir,
    )
    .err()
    .expect("computed u16 field cannot be recontextualized as u8");
    assert!(error.contains("type mismatch"), "{error}");
}

#[test]
fn template_type_list_variadics_collect_completed_type_values() {
    let source = "typealias V=.void'builtin; typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type { List<T> }
        comptime fn same(es:List<Type>)->bool { es[0] == bool and es[1] == .u8'builtin }
        template test fixture where body:<|-V>,...Es:List<Type> {
            const valid=same(Es); assumes valid; ~<body>;
        }
        test types use fixture<bool,.u8'builtin> {}";
    let Artifact::Nir(package) = compile_with_meta_list(source, &[], Emit::Nir, true).unwrap()
    else {
        unreachable!()
    };
    assert!(
        luna::testing::run_report(&package, 0, 100_000, false, None)
            .unwrap()
            .is_success()
    );
    assert!(
        compile_with_meta_list(
            &source.replace("<bool,.u8'builtin>", "<true,.u8'builtin>"),
            &[],
            Emit::Nir,
            true
        )
        .is_err()
    );
    assert!(
        compile(source, Emit::Nir).is_err(),
        "a similarly named unbound List is not the collection protocol"
    );
}

#[test]
fn template_type_list_spreads_names_and_empty_collections() {
    let source = "typealias V=.void'builtin; typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type { List<T> }
        comptime fn types()->List<Type> { [bool,.u8'builtin] }
        comptime fn same(es:List<Type>)->bool { es[0] == bool and es[1] == .u8'builtin }
        template test fixture where body:<|-V>,...Es:List<Type> {
            const valid=same(Es); assumes valid; ~<body>;
        }
        template test empty_case where body:<|-V>,...Es:List<Type> { ~<body>; }
        test spread use fixture<...types()> {}
        test literal_spread use fixture<...[bool,.u8'builtin]> {}
        test named use fixture<.Es=types()> {}
        test empty use empty_case {}";
    let Artifact::Nir(package) = compile_with_meta_list(source, &[], Emit::Nir, true).unwrap()
    else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 4);
    assert!(report.is_success(), "{report}");
    for args in [
        "...(bool,.u8'builtin)",
        ".Es=(bool,.u8'builtin)",
        "...[true,.u8'builtin]",
        "bool,.Es=types()",
    ] {
        assert!(
            compile_with_meta_list(
                &format!("{source} test bad use fixture<{args}> {{}}"),
                &[],
                Emit::Nir,
                true
            )
            .is_err(),
            "accepted {args}"
        );
    }
}

#[test]
fn template_type_list_collection_preserves_normal_form_heads() {
    let source = "typealias V=.void'builtin; typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type { List<T> }
        pure comptime fn Identity<T>->Type { T }
        comptime fn check_identity(es:List<Type>)->bool { es[0] == Identity<bool> }
        template test fixture where body:<|-V>,...Es:List<Type> {
            const valid=check_identity(Es); assumes valid; ~<body>;
        }
        test preserved use fixture<Identity<bool>> {}";
    let Artifact::Nir(package) = compile_with_meta_list(source, &[], Emit::Nir, true).unwrap()
    else {
        unreachable!()
    };
    assert!(
        luna::testing::run_report(&package, 0, 100_000, false, None)
            .unwrap()
            .is_success()
    );
    let unresolved = compile_with_meta_list(
        &source.replace("es[0] == Identity<bool>", "es[0] != bool"),
        &[],
        Emit::Nir,
        true,
    )
    .err()
    .expect("opaque head comparison must not invent a negative type fact");
    assert!(
        unresolved.contains("TypeContractUnresolved"),
        "{unresolved}"
    );
}

#[test]
fn template_multiple_tuple_entrances_use_named_packages_or_one_remaining_target() {
    let source = "typealias V=.void'builtin;
        template test fixture where body:<|-V>,...first:Tuple<bool,bool>,...second:Tuple<.u8'builtin> {
            let (a,b)=first; let (n,)=second; assumes a and not b and n == 7; ~<body>;
        }
        test named use fixture<.second=(7,),.first=(true,false)> {}
        test remaining use fixture<true,false,.second=(7,)> {}
        test spread use fixture<.first=(true,false),...(7,)> {}";
    let Artifact::Nir(package) = compile_with_tuple(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert!(
        luna::testing::run_report(&package, 0, 100_000, false, None)
            .unwrap()
            .is_success()
    );
    for args in [
        "true,false,7",
        ".first=(true,false),.second=(7,),false",
        ".first=(true,false)",
        ".second=(7,),.first=(true,7)",
    ] {
        assert!(
            compile_with_tuple(
                &format!("{source} test bad use fixture<{args}> {{}}"),
                Emit::Nir
            )
            .is_err(),
            "accepted {args}"
        );
    }
}

#[test]
fn template_mixed_collection_domains_keep_protocols_and_empty_inputs_distinct() {
    let source = "typealias V=.void'builtin; typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type { List<T> }
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn check_type(es:List<Type>)->bool { es[0] == bool }
        template test fixture where body:<|-V>,...flags:Tuple<bool>,...types:List<Type> {
            let (flag,)=flags; const valid=check_type(types); assumes flag and valid; ~<body>;
        }
        template test empty_case where body:<|-V>,...left:List<Type>,...right:List<Type> { ~<body>; }
        test type_tail use fixture<.flags=(true,),bool> {}
        test tuple_tail use fixture<true,.types=[bool]> {}
        test empty use empty_case {}";
    let Artifact::Nir(package) = compile_with_constructors(
        source,
        &[],
        Emit::Nir,
        Some("input.List"),
        Some("input.Tuple"),
    )
    .unwrap() else {
        unreachable!()
    };
    let report = luna::testing::run_report(&package, 0, 100_000, false, None).unwrap();
    assert_eq!(report.results.len(), 3);
    assert!(report.is_success(), "{report}");
    let failure = compile_with_constructors(
        &format!("{source} test ambiguous use fixture<true,bool> {{}}"),
        &[],
        Emit::Nir,
        Some("input.List"),
        Some("input.Tuple"),
    )
    .err()
    .expect("input types must not guess an ambiguous target");
    assert!(
        failure.contains("ambiguous test template variadic input"),
        "{failure}"
    );
}

#[test]
fn template_collection_routing_rejects_ambiguity_before_invoking_inputs() {
    let declaration = "typealias V=.void'builtin;
        comptime fn fail_input()->Tuple<bool,bool> { assumes false; (true,false) }
        template test fixture where body:<|-V>,...first:Tuple<bool,bool>,...second:Tuple<bool> { ~<body>; }";
    for (arguments, expected) in [
        ("...fail_input()", "ambiguous test template variadic input"),
        (
            ".first=fail_input(),.first=(true,false),.second=(true,)",
            "duplicate test template argument",
        ),
        (
            ".first=fail_input(),.second=(true,),false",
            "cannot append to directly supplied collections",
        ),
    ] {
        let failure = compile_with_tuple(
            &format!("{declaration} test rejected use fixture<{arguments}> {{}}"),
            Emit::Nir,
        )
        .err()
        .expect("malformed routing must fail before comptime invocation");
        assert!(failure.contains(expected), "{failure}");
    }
}

#[test]
fn signature_templates_clone_modified_callable_interfaces_before_admission() {
    for modifier in [
        "pure",
        "inline",
        "spec",
        "verified",
        "pure comptime inline unsafe",
    ] {
        let source = format!(
            "template fn interface(callback:{modifier} fn(comptime T:.Type'builtin,value:T)->T)->bool;fn first use interface {{true}}fn second use interface {{false}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("modified callable admission remains required");
        assert!(
            error.contains("elaboration is not implemented for function type modifiers"),
            "{modifier}: {error}"
        );
        assert!(
            !error.contains("substitution not yet implemented"),
            "{error}"
        );
    }
    let source = "template fn interface(callback:unsafe fn(comptime T:.Type'builtin,value:T)->T)->bool;fn first use interface {true}fn second use interface {false}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    for name in ["first", "second"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let nir::Term::Function { parameters, .. } = &function.signature else {
            unreachable!()
        };
        assert!(parameters[0].ty.is_unsafe_callable());
    }
}

#[test]
fn signature_template_defaults_keep_nested_tactic_boundaries() {
    let source="fn apply(value:bool,lambda action:fn(bool)->bool)->bool {action(value)}
        fn always(value:bool)->bool {true}
        template fn choose(value:bool,.action:fn(bool)->bool=|input:bool| apply(input,_0 and not apply(false,_0)))->bool;
        fn first use choose {action(value)}
        fn second use choose {action(value)}
        fn run()->bool {first(true) and not second(false) and first(false,.action=always)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(execution_support::id(&package, "run"), vec![])
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert_eq!(
        execution_support::calls(&package, &vm),
        [
            "run", "first", "apply", "apply", "second", "apply", "first", "always"
        ]
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn signature_template_tactics_still_require_a_valid_callable_boundary() {
    for default in ["_0", "apply(value,_1)"] {
        let source = format!(
            "fn apply(value:bool,lambda action:fn(bool)->bool)->bool {{action(value)}}
            template fn choose(value:bool,.selected:bool={default})->bool;
            fn first use choose {{selected}}
            fn run()->bool {{first(true)}}"
        );
        let error = compile(&source, Emit::Nir).err().unwrap();
        assert!(
            !error.contains("signature requires substitution not yet implemented"),
            "{error}"
        );
        assert!(
            error.contains("tactic") || error.contains("placeholder"),
            "{error}"
        );
    }
}

#[test]
fn signature_template_tactics_rebind_captured_default_inputs() {
    let source="fn apply(input:bool,lambda action:%{value:bool} fn(bool)->bool)->bool {action(input)}
        template fn choose(value:bool,.action:%{value:bool} fn(bool)->bool=|input:bool| apply(input,_0 and value))->bool;
        fn first use choose {action(true)}
        fn second use choose {action(true)}
        fn run()->bool {first(true) and not second(false) and first(true)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn signature_template_defaults_rebind_contextual_code_and_pattern_values() {
    for default in [
        "do {const code:<|-bool> = '(true); ~<code>}",
        "do {const inner:<|-bool> = '(false); const code:<|-bool> = '(not ~<inner>); ~<code>}",
        "do {const code:<flag:bool|-bool> = '(not flag); ~[flag=seed]<code>}",
        "do {const code:<seed:bool|-bool> = '(not seed); ~<code>}",
        "do {const pattern:<bool -| bound:bool> = '<bound>; seed match {~<pattern> => not bound}}",
    ] {
        let opposite = if default.contains("not flag")
            || default.contains("not seed")
            || default.contains("not bound")
        {
            " and not first(true) and second(true)"
        } else {
            ""
        };
        let source=format!("template fn choose(seed:bool,.value:bool={default})->bool;
            fn first use choose {{value}} fn second use choose {{not value}}
            fn run()->bool {{first(false) and not second(false) and not first(false,.value=false){opposite}}}");
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|e| panic!("{source}: {e}"))
        else {
            unreachable!()
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}

#[test]
fn signature_template_quotes_keep_holes_and_lexical_port_requirements() {
    for (default, expected) in [
        (
            "do {const inner:<|-bool> = '(true); const code:<|-bool> = '(~<inner> and ~<inner>); ~<code>}",
            "at most one splice",
        ),
        (
            "do {const code:<missing:bool|-bool> = '(missing); ~<code>}",
            "binding",
        ),
    ] {
        let source = format!(
            "template fn choose(.value:bool={default})->bool;
            fn first use choose {{let missing=true; value}}
            fn run()->bool {{first()}}"
        );
        let error = compile(&source, Emit::Nir).err().unwrap();
        assert!(
            !error.contains("substitution not yet implemented"),
            "{error}"
        );
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn signature_templates_preserve_declared_unsafe_call_boundaries() {
    let source = "template unsafe fn boundary(value:bool)->bool;
        fn first use boundary {value}
        inline fn second use boundary {not value}
        quote fn third use boundary {value}
        fn invoke(action:unsafe fn(bool)->bool)->bool {unsafe {action(true)}}
        fn run()->bool {unsafe {first(true) and second(false) and third(true) and invoke(first)}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        unreachable!()
    };
    for name in ["first", "second"] {
        let function = package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        assert!(
            function.is_unsafe,
            "template call boundary disappeared from {name}"
        );
        assert!(function.callable_type().is_unsafe_callable());
    }
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
    for call in ["first(true)", "second(false)", "third(true)"] {
        let invalid = source.replace(
            "unsafe {first(true) and second(false) and third(true) and invoke(first)}",
            call,
        );
        let error = compile(&invalid, Emit::Nir)
            .err()
            .unwrap_or_else(|| panic!("unsafe boundary lost: {call}"));
        assert!(error.contains("unsafe"), "{error}");
    }
    let invalid = source.replace("action:unsafe fn(bool)->bool", "action:fn(bool)->bool");
    assert!(compile(&invalid, Emit::Nir).is_err());
}

#[test]
fn signature_template_instance_modifiers_preserve_interface_contracts() {
    for (template_modifiers, instance_modifiers, pure, unsafe_call) in [
        ("pure", "unsafe", true, true),
        ("unsafe", "pure", true, true),
        ("pure unsafe", "", true, true),
        ("", "", false, false),
    ] {
        let source = format!(
            "template {template_modifiers} fn boundary(value:bool)->bool;
            {instance_modifiers} fn instance use boundary {{value}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            unreachable!()
        };
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "instance")
            .unwrap();
        assert_eq!(function.pure, pure);
        assert_eq!(function.is_unsafe, unsafe_call);
    }
}
