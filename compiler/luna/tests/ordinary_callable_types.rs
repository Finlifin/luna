//! Computed Type clauses retain their producers until callable formation.
use comptime_interpretor::{Budget, Value};
#[path = "support/sysroot_execution.rs"]
mod sysroot;

fn check_both(source: &str) {
    let package = sysroot::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|function| function.name == "check")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        sysroot::with_comptime(&package, Budget::default(), tree, |vm| {
            assert_eq!(
                vm.invoke(entry, vec![]).unwrap(),
                Value::Bool(true),
                "tree={tree}"
            );
        });
    }
}

#[test]
fn selected_runtime_method_keeps_a_type_valued_member_input() {
    let source = "
        trait Probe { fn read() -> bool where D, field: D; }
        impl Probe for bool { fn read() -> bool where D, field: D { true } }
        comptime fn select() -> fn() -> bool { bool.read<Type, u8> }
    ";
    let package = sysroot::compile(source).unwrap();
    let select = package
        .functions
        .iter()
        .find(|f| f.name == "select")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, tree, |vm| {
            let Value::RuntimeTraitMethod(method) = vm.invoke(select, vec![]).unwrap() else {
                panic!("expected the public method identity");
            };
            assert_eq!(
                method.method_arguments,
                [nir::Primitive::Type, nir::Primitive::U8].map(nir::Term::Primitive)
            );
        });
    }
}

#[test]
fn recursive_clause_calls_keep_caller_identities_out_of_callee_dependencies() {
    let source = "
        comptime fn Byte() -> Type { u8 }
        comptime fn rotate(again: bool) -> ?A where A, B, flag: bool {
            if again { let ignored = rotate<B, Byte(), false>(false); };
            null
        }
        comptime fn check() -> bool {
            let value = rotate<u16, bool, true>(true);
            value matches null
        }
    ";
    check_both(source);
    // With an actual B = bool the recursive result is ?bool. An unrelated
    // pending slot cannot make that result admissible as an arbitrary type.
    let wrong = source.replace("let ignored = rotate", "let ignored: u32 = rotate");
    assert!(sysroot::compile(&wrong).is_err());
}

#[test]
fn recursive_domain_proofs_use_the_inner_invocations_pending_subjects() {
    let source = "
        comptime fn computed(t: Type) -> Type { t }
        comptime fn swap(witness: U, again: bool) -> bool
        where T, U, value: T, requires T != U
        {
            if again { swap<computed(U), T, witness>(value, false) } else { true }
        }
        comptime fn check() -> bool {
            let byte: u8 = 3;
            swap<bool, u8, true>(byte, true)
        }
    ";
    check_both(source);
    // The callee's computed T is u8 here. A negative proof that found no
    // conflict during elaboration cannot admit an actual bool at invocation.
    let invalid = source.replace("T, witness>", "T, value>");
    let package = sysroot::compile(&invalid).unwrap();
    let check = package
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    for reference in [false, true] {
        sysroot::with_comptime(&package, Budget::default(), reference, |vm| {
            let err = vm.invoke(check, vec![]).unwrap_err();
            assert!(
                matches!(
                    err.kind,
                    comptime_interpretor::ErrorKind::FunctionInputRejected(_)
                ),
                "{err:?}"
            );
        });
    }
}

#[test]
fn pending_value_indices_remain_local_in_recursive_nominal_domains() {
    check_both(
        "
        struct Ticket where index: bool {}
        comptime fn computed(value: bool) -> bool { value }
        comptime fn swap(again: bool) -> bool
        where a: bool, b: bool, first: Ticket<a>, second: Ticket<b>,
              requires Ticket<a> != Ticket<b>
        {
            if again { swap<computed(b), a, second, first>(false) } else { true }
        }
        comptime fn check() -> bool {
            swap<true, false, Ticket<true> {}, Ticket<false> {}>(true)
        }
    ",
    );
}

#[test]
fn pending_subjects_preserve_repeated_positions_and_caller_disequality() {
    let source = "
        struct Pair where A, B {}
        comptime fn computed(t: Type) -> Type { t }
        comptime fn same() -> bool where T, pair: Pair<T, T> { true }
        comptime fn unused() -> bool where A, B, requires A != B {
            let pair = Pair<A, B> {};
            same<computed(A), pair>()
        }
    ";
    let err = sysroot::compile(source).unwrap_err();
    assert!(err.contains("type mismatch"), "{err}");
}

#[test]
fn selected_parent_inputs_and_member_supplies_substitute_simultaneously() {
    let source = "
        struct Holder where Outer {
            pub comptime fn recurse(witness: A) -> bool
            where A, value: Outer, again: bool
            {
                if again { Holder<A>.recurse<bool, witness, false>(true) } else { true }
            }
        }
        comptime fn check() -> bool {
            let byte: u8 = 3;
            Holder<bool>.recurse<u8, true, true>(byte)
        }
    ";
    check_both(source);
    let wrong = source.replace(
        "recurse<bool, witness, false>",
        "recurse<bool, true, false>",
    );
    assert!(sysroot::compile(&wrong).is_err());
}

#[test]
fn early_default_domain_checks_restore_caller_before_later_explicit_inputs() {
    check_both(
        "
        struct Holder where seed: bool {
            pub comptime fn pick() -> bool
            where .T: Type = u8, index: T, value: bool
            {
                value == seed
            }
            pub comptime fn evaluate() -> bool {
                var trace: Integer = 0;
                let yes = Holder<true>.pick<do { trace = trace * 10 + 1; 7 }, seed>;
                let no = Holder<false>.pick<do { trace = trace * 10 + 2; 8 }, seed>;
                yes() == seed and no() != seed and trace == 12
            }
        }
        comptime fn check() -> bool {
            Holder<true>.evaluate() and Holder<false>.evaluate()
        }
    ",
    );
}

#[test]
fn sibling_callable_defaults_keep_selected_parent_and_caller_inputs_separate() {
    let source = "
        struct Holder where seed: bool {
            pub comptime fn pick() -> bool
            where value: bool, .copied: bool = seed, .T: Type = u8, requires T == u8
            {
                value == copied
            }
            pub comptime fn evaluate() -> bool {
                var trace: Integer = 0;
                let yes = Holder<true>.pick<do { trace = trace * 10 + 1; seed }>;
                let no = Holder<false>.pick<do { trace = trace * 10 + 2; seed }>;
                yes() == seed and no() != seed and yes() == seed and trace == 12
            }
        }
        comptime fn check() -> bool {
            Holder<true>.evaluate() and Holder<false>.evaluate()
        }
    ";
    check_both(source);
    // Formation remains mandatory even for an unused callable. A previous
    // successful sibling must not supply evidence for this selected parent.
    let invalid = source.replace(
        "let no = Holder<false>.pick<do { trace = trace * 10 + 2; seed }>;",
        "let unused = Holder<false>.pick<seed, .T = bool>;\nlet no = Holder<false>.pick<seed>;",
    );
    let failure = sysroot::compile(&invalid).unwrap_err();
    assert!(
        failure.contains("type requirement is not satisfied"),
        "{failure}"
    );
    check_both(source);
}

#[test]
fn nominal_type_inputs_form_callables_without_executing_the_input_plan() {
    let source = "
        struct Key where T, value: T, .other: T = value { payload: bool }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let selected = identity<do {
                trace = trace * 10 + 1;
                Key<.other = bool, Type, bool>
            }>;
            let first = selected(Key<Type, bool> { payload: true });
            let second = selected(Key<Type, bool> { payload: false });
            first.payload and not second.payload and trace == 1
        }
    ";
    check_both(source);
    let wrong = source.replace(
        "let second = selected(Key<Type, bool>",
        "let second = selected(Key<Type, u8>",
    );
    assert!(sysroot::compile(&wrong).is_err());
}

#[test]
fn nominal_type_input_observation_preserves_opaque_indices_and_formation() {
    let source = "
        pure comptime fn Opaque<T> -> Type { T }
        struct Key where T, value: T, requires T == Type { payload: bool }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn check() -> bool {
            let selected = identity<Key<Type, Opaque<u8>>>;
            selected(Key<Type, Opaque<u8>> { payload: true }).payload
        }
    ";
    check_both(source);
    let unfolded = source.replace("selected(Key<Type, Opaque<u8>>", "selected(Key<Type, u8>");
    assert!(sysroot::compile(&unfolded).is_err());
    let invalid = source.replace("Key<Type, Opaque<u8>>", "Key<bool, true>");
    assert!(sysroot::compile(&invalid).is_err());
}

#[test]
fn computed_types_with_independent_clause_domains_form_closed_callables() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn Byte() -> Type { u8 }
        comptime fn choose() -> bool where T, flag: bool { flag }
        fn run() -> bool { const result = choose<Byte(), true>(); result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn computed_type_producers_keep_order_and_callable_values_do_not_replay_them() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn choose() -> bool where T, flag: bool { flag }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let selected = choose<
                do { trace = trace * 10 + 1; u8 },
                do { trace = trace * 10 + 2; true }
            >;
            selected() and selected() and trace == 12
        }
        fn run() -> bool { const result = check(); result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn computed_type_identity_and_type_defaults_reach_the_selected_body() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn Byte() -> Type { u8 }
        comptime fn choose() -> bool
        where T, .U: Type = T, .flag: bool = true
        { T == u8 and U == T and flag }
        fn run() -> bool { const result = choose<Byte()>(); result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn named_inputs_before_computed_types_keep_their_source_order() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn choose() -> bool where T, .flag: bool = false { flag and T == u8 }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let selected = choose<
                .flag = do { trace = trace * 10 + 1; true },
                do { trace = trace * 10 + 2; u8 }
            >;
            selected() and trace == 12
        }
        fn run() -> bool { const result = check(); result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn early_return_from_a_type_input_skips_later_producers_and_formation() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn choose() -> bool where T, flag: bool { flag }
        comptime fn check() -> bool {
            let selected = choose<
                do { return true; u8 },
                do { let poison = 1.div_trunc(0); false }
            >;
            false
        }
        fn run() -> bool { const result = check(); result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn computed_type_contracts_are_checked_before_unused_callable_values_escape() {
    let source = |ty| {
        format!(
            "
        comptime fn Selected() -> Type {{ {ty} }}
        comptime fn guarded() -> bool
        where T, flag: bool, requires T == u8
        {{ flag }}
        comptime fn check() -> bool {{ let selected = guarded<Selected(), true>; true }}
        fn run() -> bool {{ const result = check(); result }}
    "
        )
    };
    assert_eq!(sysroot::run(&source("u8")), Value::Bool(true));
    let failure = sysroot::compile(&source("u16")).unwrap_err();
    assert!(failure.contains("TypeContractRejected"), "{failure}");
}

#[test]
fn computed_types_keep_opaque_normal_form_identity() {
    assert_eq!(
        sysroot::run(
            "
        pure comptime fn Identity<T> -> Type { T }
        comptime fn Selected() -> Type { Identity<u8> }
        comptime fn choose() -> bool where T, flag: bool { flag and T == Identity<u8> }
        fn run() -> bool { const result = choose<Selected(), true>(); result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn runtime_callable_input_preparation_executes_only_the_clause_packet() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn Byte() -> Type { u8 }
        fn choose() -> bool where T, ...flags: meta.List<bool> {
            inline if flags.length() == 2 { true } else { false }
        }
        fn run() -> bool { choose<Byte(), true, false>() }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
#[ignore = "a callable signature depending on an invocation-time Type still needs contextual checking"]
fn computed_type_inputs_can_complete_a_dependent_callable_signature() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn Byte() -> Type { u8 }
        comptime fn identity(value: T) -> T where T, flag: bool { value }
        fn run() -> bool { const result = identity<Byte(), true>(9) == 9; result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn an_unfinished_signature_cannot_be_published_as_an_unrelated_callable_type() {
    let failure = sysroot::compile(
        "
        comptime fn Byte() -> Type { u8 }
        comptime fn identity(value: T) -> T where T, flag: bool { value }
        comptime fn check() -> bool {
            let selected: comptime fn(bool) -> bool = identity<Byte(), true>;
            true
        }
        fn run() -> bool { const result = check(); result }
    ",
    )
    .unwrap_err();
    assert!(
        failure.contains("signature still depends on an invocation-time clause input"),
        "{failure}"
    );
}

#[test]
fn type_only_clause_packets_keep_computed_type_producers() {
    assert_eq!(
        sysroot::run(
            "
        comptime fn choose() -> bool where T { T == u8 }
        comptime fn check() -> bool {
            var called = false;
            let selected = choose<do { called = true; u8 }>;
            selected() and called
        }
        fn run() -> bool { const result = check(); result }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn callable_values_snapshot_mutable_type_inputs_in_both_engines() {
    check_both(
        "
        comptime fn choose() -> bool where T, flag: bool { T == u8 and flag }
        comptime fn check() -> bool {
            var kind: Type = u8;
            let selected = choose<kind, true>;
            kind = u16;
            selected() and selected() and kind == u16
        }
        ",
    );
}

#[test]
fn observed_type_producers_retain_dependent_parameter_defaults() {
    check_both(
        "
        comptime fn identity(input: T, .result: T = input) -> T where T { result }
        comptime fn check() -> bool {
            identity<do { let kind = bool; kind }>(true)
        }
        ",
    );
}

#[test]
fn immutable_input_evidence_completes_signatures_without_unfolding_heads() {
    check_both(
        "
        pure comptime fn Indexed<...items: meta.List<bool>> -> Type { bool }
        struct Holder where T { flag: bool }
        comptime fn accept(value: Holder<T>) -> bool where T { value.flag }
        comptime fn check() -> bool {
            let bits: meta.List<bool> = [true, false];
            accept<Indexed<.items = bits>>(Holder<Indexed<true, false>> { flag: true })
        }
        ",
    );
}

#[test]
fn dependency_checks_preserve_source_effects_and_skip_unreached_type_producers() {
    check_both(
        "
        comptime fn count() -> Integer where T, ...values: meta.List<T> {
            values.length()
        }
        comptime fn ordered() -> bool {
            var trace: Integer = 0;
            let selected = count<
                .values = do { trace = trace * 10 + 1; [1, 255] },
                do { trace = trace * 10 + 2; u8 }
            >;
            selected() == 2 and selected() == 2 and trace == 12
        }
        comptime fn exits() -> bool {
            let selected = count<
                .values = do { return true; [1, 255] },
                do { let poison = 1.div_trunc(0); u8 }
            >;
            false
        }
        comptime fn check() -> bool { ordered() and exits() }
        ",
    );
}

#[test]
fn value_indexed_signatures_keep_clause_effects_and_the_selected_identity() {
    let source = "
        struct Flag where tag: bool { value: bool }
        comptime fn read(value: Flag<tag>) -> bool where tag: bool { value.value }
        comptime fn check() -> bool {
            var visits: Integer = 0;
            let get = read<do { visits = visits + 1; true }>;
            let value = Flag<true> { value: true };
            get(value) and get(value) and visits == 1
        }
    ";
    check_both(source);
    assert!(
        sysroot::compile(&source.replace("let value = Flag<true>", "let value = Flag<false>"))
            .is_err()
    );
}

#[test]
fn default_parameters_receive_value_indices_from_their_callable() {
    let source = "
        struct Flag where tag: bool { value: bool }
        comptime fn read(comptime .T: Type = Flag<tag>, value: T) -> bool
        where tag: bool
        {
            true
        }
        comptime fn check() -> bool {
            var visits: Integer = 0;
            let first = read<do { visits = visits + 1; true }>(Flag<true> { value: true });
            first and read<do { visits = visits + 1; false }>(Flag<false> { value: true })
                and visits == 2
        }
    ";
    check_both(source);
    let mismatch = source.replace("Flag<false> { value: true }", "Flag<true> { value: true }");
    let failure = sysroot::compile(&mismatch).unwrap_err();
    assert!(failure.contains("type mismatch"), "{failure}");
}
