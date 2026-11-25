//! End-to-end tests for OQ-3 (`...` rest patterns), HIR pattern orphans
//! (`PatternKind::{Ref, Async}`), and take views (`'multi_deref`, `'type`,
//! `'tuple_enumerate`, `'tuple_indexed`).
#[path = "support/execution.rs"]
mod support;

use comptime_interpretor::{Event, Interpreter, Value};
use elaboration::{
    capabilities::World,
    implementation::{ImplementationHead, Index},
    program::Protocols,
    ptc::{Budget, Context},
};
use intrinsic::{LangItem, LangItems};
use luna::protocols::type_constructors::Selectors;

fn drop_selectors() -> Selectors<'static> {
    Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    }
}

fn compile_with_drop(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(source, &Protocols::default(), drop_selectors())
}

fn run_with_drop(source: &str, entry: &str) -> (Value, Vec<String>) {
    let package = compile_with_drop(source).unwrap();
    let context = Context::from_package(&package).unwrap();
    let mut index = Index::default();
    for implementation in package
        .implementations
        .iter()
        .filter(|implementation| implementation.interface.is_some())
    {
        index
            .insert(
                ImplementationHead {
                    parameter_types: implementation.parameter_types.clone(),
                    declaration: implementation.declaration,
                    interface: implementation.interface,
                    interface_arguments: implementation.interface_arguments.clone(),
                    subject: implementation.subject.clone(),
                    type_parameters: implementation.type_parameters.clone(),
                    requirements: implementation.requirements.clone(),
                },
                Budget::default(),
            )
            .unwrap();
    }
    let world = World {
        index: &index,
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: &[],
        context: &context,
    };
    let mut items = LangItems::new();
    items
        .bind_source_traits(
            &package,
            &[
                (
                    LangItem::Drop,
                    package
                        .traits
                        .iter()
                        .find(|t| t.name == "Drop")
                        .unwrap()
                        .declaration,
                ),
                (
                    LangItem::Copy,
                    package
                        .traits
                        .iter()
                        .find(|t| t.name == "Copy")
                        .unwrap()
                        .declaration,
                ),
            ],
        )
        .unwrap();
    let entry_key = iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .declaration,
        arguments: vec![].into(),
    };
    let executable = luna::cleanup::prepare_source(
        &package,
        &world,
        &items,
        entry_key,
        &mut 100_000,
        Budget::default(),
    )
    .unwrap();
    let mut vm = Interpreter::from_iir(&executable.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    let result = vm.invoke_instance(&executable.entry, vec![]).unwrap();
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(
                executable
                    .graph
                    .functions
                    .iter()
                    .find(|f| f.key.declaration == *id)
                    .unwrap()
                    .function
                    .name
                    .clone(),
            ),
            _ => None,
        })
        .collect();
    (result, calls)
}

#[test]
fn struct_record_enum_three_dot_rest_patterns_work_and_reject_invalid_rest() {
    let valid = "
        comptime fn schema(...properties: T) -> T where T { properties }
        const SAMPLE = schema { a: true, b: false };
        typealias AnonPair = SAMPLE'type;
        fn build(...properties: T) -> T where T { properties }

        struct Point { x: bool, y: bool, z: bool }
        enum Msg {
            data { flag: bool, extra: bool },
            empty,
        }

        fn read_point(p: Point) -> bool {
            let Point { x, ... } = p;
            x
        }

        fn read_anon(r: AnonPair) -> bool {
            let { a, ... } = r;
            a
        }

        fn read_msg(m: Msg) -> bool {
            m match {
                .data { flag, ... } => flag,
                .empty => false,
            }
        }

        fn read_omitted_wildcard(p: Point, r: AnonPair, m: Msg) -> bool {
            let Point { x } = p;
            let { a } = r;
            let mflag = m match {
                .data { flag } => flag,
                .empty => false,
            };
            x and a and mflag
        }

        fn run() -> bool {
            let p = Point { x: true, y: false, z: false };
            let r = build { a: true, b: false };
            let m = Msg.data { flag: true, extra: false };
            read_point(p)
                and read_anon(r)
                and read_msg(m)
                and read_omitted_wildcard(p, r, m)
        }
    ";
    assert_eq!(support::run_iir(valid, "run", vec![]), Value::Bool(true));

    for (bad, needle) in [
        (
            "struct Point { x: bool, y: bool } fn f(p: Point) -> bool { let Point { x, .. } = p; x }",
            "use `...` (three dots) instead of `..`",
        ),
        (
            "struct Point { x: bool, y: bool } fn f(p: Point) -> bool { let Point { ..., x } = p; x }",
            "must be the last entry",
        ),
        (
            "struct Point { x: bool, y: bool } fn f(p: Point) -> bool { let Point { x, ..., ... } = p; x }",
            "duplicate `...`",
        ),
    ] {
        let err = support::compile_with_protocols(bad, &Protocols::default()).unwrap_err();
        assert!(err.contains(needle), "expected `{needle}` in: {err}");
    }
}

#[test]
fn rest_pattern_enforces_once_cleanup_and_forbids_partial_move_under_drop() {
    let prefix = "
        trait Copy {}
        impl Copy for bool {}
        trait Drop { fn drop(*self); }
        struct Ticket { flag: bool }
        impl Drop for Ticket { fn drop(*self) {} }
    ";

    // Without custom Drop on the outer container, omitting a Once field under `...`
    // is allowed and runs the omitted field's Drop cleanup exactly once.
    let allow_omitted_once_without_outer_drop = format!(
        "{prefix}
        struct Bag {{ keep: bool, omitted: Ticket }}
        fn take_keep(bag: Bag) -> bool {{
            let Bag {{ keep, ... }} = bag;
            keep
        }}
        fn run() -> bool {{
            take_keep(Bag {{ keep: true, omitted: Ticket {{ flag: false }} }})
        }}"
    );
    let (value, calls) = run_with_drop(&allow_omitted_once_without_outer_drop, "run");
    assert_eq!(value, Value::Bool(true));
    assert_eq!(
        calls.iter().filter(|name| name.as_str() == "drop").count(),
        1,
        "expected omitted Ticket::drop to run once: {calls:?}"
    );

    // With custom Drop on the outer struct, moving a non-Copy field under `...` is rejected.
    let reject_struct_partial_move = format!(
        "{prefix}
        struct Guarded {{ ticket: Ticket, flag: bool }}
        impl Drop for Guarded {{ fn drop(*self) {{}} }}
        fn bad(g: Guarded) -> Ticket {{
            let Guarded {{ ticket, ... }} = g;
            ticket
        }}"
    );
    let err = compile_with_drop(&reject_struct_partial_move).unwrap_err();
    assert!(
        err.contains(
            "cannot partially move non-Copy field from a type that implements Drop under `...`"
        ),
        "{err}"
    );

    // With custom Drop on an outer enum, moving a non-Copy named variant field under `...` is rejected.
    let reject_enum_partial_move = format!(
        "{prefix}
        enum GuardedEnum {{ pair {{ ticket: Ticket, flag: bool }} }}
        impl Drop for GuardedEnum {{ fn drop(*self) {{}} }}
        fn bad(g: GuardedEnum) -> Ticket {{
            g match {{
                .pair {{ ticket, ... }} => ticket,
            }}
        }}"
    );
    let err = compile_with_drop(&reject_enum_partial_move).unwrap_err();
    assert!(
        err.contains(
            "cannot partially move non-Copy field from a type that implements Drop under `...`"
        ),
        "{err}"
    );

    // Binding only a Copy field under `...` on a Drop struct keeps the parent intact for Drop.
    let allow_copy_field_with_drop = format!(
        "{prefix}
        struct Guarded {{ ticket: Ticket, flag: bool }}
        impl Drop for Guarded {{ fn drop(*self) {{}} }}
        fn read_flag(g: Guarded) -> bool {{
            let Guarded {{ flag, ... }} = g;
            flag
        }}
        fn run() -> bool {{
            read_flag(Guarded {{ ticket: Ticket {{ flag: false }}, flag: true }})
        }}"
    );
    let (value, calls) = run_with_drop(&allow_copy_field_with_drop, "run");
    assert_eq!(value, Value::Bool(true));
    assert_eq!(
        calls.iter().filter(|name| name.as_str() == "drop").count(),
        2,
        "expected Guarded and Ticket drops to run: {calls:?}"
    );
}

#[test]
fn ref_and_async_patterns_resolve_correctly_or_emit_clear_diagnostics() {
    let ref_ok = "
        struct Boxed { flag: bool }
        fn read_ptr(p: *bool) -> bool {
            let ref x = p;
            x
        }
        fn read_nested(pp: **bool) -> bool {
            let ref (ref x) = pp;
            x
        }
        fn match_nested(pp: **bool) -> bool {
            pp match {
                ref (ref true) => true,
                _ => false,
            }
        }
        fn read_once_ptr_chain(pp: **Boxed) -> bool {
            let ref inner = pp;
            inner.*.flag
        }
        fn run() -> bool {
            let b = true;
            let p = b.ref;
            let pp = p.ref;
            let box = Boxed { flag: true };
            let bp = box.ref;
            let bpp = bp.ref;
            read_ptr(p) and read_nested(pp) and match_nested(pp) and read_once_ptr_chain(bpp)
        }
    ";
    assert_eq!(support::run_iir(ref_ok, "run", vec![]), Value::Bool(true));

    let non_ptr_ref = "fn bad(b: bool) -> bool { let ref x = b; x }";
    let err = support::compile_with_protocols(non_ptr_ref, &Protocols::default()).unwrap_err();
    assert!(
        err.contains("ref pattern requires a pointer subject"),
        "{err}"
    );

    let non_copy_ref = "
        trait Copy {}
        impl Copy for bool {}
        trait Drop { fn drop(*self); }
        struct OnceBox { flag: bool }
        impl Drop for OnceBox { fn drop(*self) {} }
        fn bad(p: *OnceBox) -> OnceBox { let ref x = p; x }
    ";
    let err = compile_with_drop(non_copy_ref).unwrap_err();
    assert!(
        err.contains("pointer reads of non-intrinsic values require Copy evidence")
            || err.contains("Copy"),
        "{err}"
    );

    let async_pat = "fn bad(b: bool) -> bool { b match { async x => x } }";
    let err = support::compile_with_protocols(async_pat, &Protocols::default()).unwrap_err();
    assert!(err.contains("async patterns are not supported"), "{err}");
}

#[test]
fn multi_deref_view_supports_chains_places_reborrow_and_generics() {
    let valid = "
        trait Copy {}
        impl Copy for bool {}
        struct Pair { left: bool, right: bool }

        fn read_generic(pp: **T) -> T where T :- Copy {
            pp'multi_deref
        }

        fn run() -> bool {
            var a = false;
            var p1 = a.ref;
            var p2 = p1.ref;
            var p3 = p2.ref;
            p3'multi_deref = true;
            let read_back = p3'multi_deref;

            var pair = Pair { left: false, right: true };
            var pp1 = pair.ref;
            var pp2 = pp1.ref;
            pp2'multi_deref.left = true;
            let reborrowed: *Pair = pp2'multi_deref.ref;
            let field_ok = reborrowed.*.left and pp2'multi_deref.right;

            var g = true;
            var gp1 = g.ref;
            var gp2 = gp1.ref;
            let gen_ok = read_generic(gp2);

            a and read_back and field_ok and gen_ok
        }
    ";
    let copy_only = Selectors {
        copy: Some("input.Copy"),
        ..Default::default()
    };
    let package =
        support::compile_with_source_bindings(valid, &Protocols::default(), copy_only).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );

    let non_ptr = "fn bad(b: bool) -> bool { b'multi_deref }";
    let err = support::compile_with_protocols(non_ptr, &Protocols::default()).unwrap_err();
    assert!(err.contains("multi_deref requires a pointer"), "{err}");

    let non_copy = "
        trait Copy {}
        fn bad(pp: **T) -> T where T { pp'multi_deref }
    ";
    let err = support::compile_with_source_bindings(non_copy, &Protocols::default(), copy_only)
        .unwrap_err();
    assert!(
        err.contains("pointer reads of non-intrinsic values require Copy evidence"),
        "{err}"
    );
}

#[test]
fn value_position_typeof_does_not_consume_once_ownership() {
    let source = "
        trait Copy {}
        impl Copy for bool {}
        trait Drop { fn drop(*self); }
        struct Token { flag: bool }
        impl Drop for Token { fn drop(*self) {} }

        comptime fn same(a: .Type'builtin, b: .Type'builtin) -> bool { a == b }

        comptime fn check_comptime() -> bool {
            let tok = Token { flag: true };
            let t = tok'type;
            let t_ok = same(t, Token);
            t_ok and tok.flag
        }

        fn consume(t: Token) -> bool { t.flag }

        fn run() -> bool {
            let tok = Token { flag: true };
            const T1 = tok'type;
            const OK_TYPE = same(T1, Token);
            const OK_COMPTIME = check_comptime();
            let ok_move = consume(tok);
            OK_COMPTIME and OK_TYPE and ok_move
        }
    ";
    let (value, calls) = run_with_drop(source, "run");
    assert_eq!(value, Value::Bool(true));
    assert_eq!(
        calls.iter().filter(|name| name.as_str() == "drop").count(),
        1,
        "expected Token::drop to run once after consume: {calls:?}"
    );
}

#[test]
fn tuple_enumerate_and_tuple_indexed_work_for_mixed_ct_rt_tuples_in_inline_for() {
    let source = "
        comptime fn check_comptime() -> bool {
            let mixed = (bool, true);
            var ok_ty = false;
            var ok_val = false;
            inline for (idx, elem) in mixed'tuple_indexed {
                inline if elem'type == .Type'builtin {
                    ok_ty = idx == 0 and elem == bool;
                } else {
                    inline if elem'type == bool {
                        ok_val = idx == 1 and elem;
                    }
                }
            }
            var count = 0;
            inline for elem in mixed'tuple_enumerate {
                let _ = elem;
                count = count + 1;
            }
            ok_ty and ok_val and count == 2
        }

        fn check_runtime(rt: bool, n: .i32'builtin) -> bool {
            let het = (n, rt);
            var enum_matched = false;
            var enum_count = 0;
            inline for elem in het'tuple_enumerate {
                inline if elem'type == bool {
                    enum_matched = elem;
                }
                enum_count = enum_count + 1;
            }
            var indexed_matched = false;
            var idx_sum = 0;
            inline for (idx, elem) in het'tuple_indexed {
                inline if elem'type == bool {
                    inline if idx == 1 {
                        indexed_matched = elem;
                    }
                }
                inline if idx == 1 {
                    idx_sum = idx_sum + 2;
                } else {
                    idx_sum = idx_sum + 1;
                }
            }
            enum_matched and enum_count == 2 and indexed_matched and idx_sum == 3
        }

        comptime fn gen_from_mixed(T: .Type'builtin, seed: bool, flag: bool) -> <|-bool> {
            let mixed = ((T, seed), flag);
            var enum_count = 0;
            inline for elem in mixed'tuple_enumerate {
                let _ = elem;
                enum_count = enum_count + 1;
            }
            var out = '(false);
            inline for (1, elem) in mixed'tuple_indexed {
                let ok = enum_count == 2;
                out = '(ok and elem);
            }
            out
        }

        fn run() -> bool {
            const OK_CT = check_comptime();
            OK_CT and check_runtime(true, 7) and ~<gen_from_mixed(bool, true, true)>
        }
    ";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );

    let bad_outside = "fn bad() { let tup = (true, false); let v = tup'tuple_indexed; }";
    let err =
        support::compile_with_tuple_protocols(bad_outside, &Protocols::default()).unwrap_err();
    assert!(
        err.contains("tuple_indexed structural iteration requires inline for"),
        "{err}"
    );
}
