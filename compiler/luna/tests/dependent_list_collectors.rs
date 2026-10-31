//! Element contexts obtained by invocation remain available to variadic collection.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn invoked_element_type_contextualizes_each_collected_literal() {
    check_invoked_element_type_context(false);
    check_invoked_element_type_context(true);
}

fn check_invoked_element_type_context(defaulted: bool) {
    let source = "typealias Type=.Type'builtin;
        mod meta {pure comptime fn List<T>->Type {T}}
        pure comptime fn Packet<T,...values:meta.List<T>>->Type {T}
        comptime fn Byte()->Type {.u8'builtin}
        comptime fn bytes()->meta.List<.u8'builtin> {[1,255]}
        comptime fn flags()->meta.List<bool> {[true]}
        comptime fn Direct()->Type {Packet<.u8'builtin,1,255>}
        comptime fn EmptyDirect()->Type {Packet<.u8'builtin>}
        comptime fn Empty()->Type {Packet<Byte()>}
        comptime fn Spread()->Type {Packet<Byte(),...bytes()>}
        comptime fn Whole()->Type {Packet<Byte(),.values=[1,255]>}
        comptime fn Overflow()->Type {Packet<Byte(),256>}
        comptime fn Wrong()->Type {Packet<Byte(),true>}
        comptime fn WrongSpread()->Type {Packet<Byte(),...flags()>}
        comptime fn Collected()->Type {Packet<Byte(),1,255>}";
    let source = if defaulted {
        format!(
            "{}\n
            comptime fn first()->.u8'builtin {{1}}
            comptime fn second()->.u8'builtin {{255}}
            comptime fn mark()->bool {{true}}
            comptime fn choose()->bool {{true}}
            comptime fn Structured()->Type {{
                Packet<Byte(),if choose() {{first()}} else {{second()}},
                    .tag=mark(),do {{second()}}>
            }}
            comptime fn StructuredFalse()->Type {{
                Packet<Byte(),if not choose() {{second()}} else {{first()}},
                    .tag=mark(),do {{second()}}>
            }}
            comptime fn Matched()->Type {{
                Packet<Byte(),choose() match {{true => first(), false => second()}},
                    .tag=mark(),do {{second()}}>
            }}
            comptime fn WrongStructured()->Type {{
                Packet<Byte(),if choose() {{mark()}} else {{false}},.tag=mark()>
            }}
            comptime fn Ordered()->Type {{Packet<Byte(),first(),.tag=mark(),second()>}}",
            source.replace("Packet<T,...values", "Packet<T,.tag:bool=true,...values")
        )
    } else {
        source.to_owned()
    };
    let package = support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
    .unwrap();
    let direct = luna::invocation::invoke(&package, "Direct", 100_000).unwrap();
    for entry in ["Collected", "Spread", "Whole"] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            direct
        );
    }
    assert_eq!(
        luna::invocation::invoke(&package, "Empty", 100_000).unwrap(),
        luna::invocation::invoke(&package, "EmptyDirect", 100_000).unwrap()
    );
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "Spread"), vec![]).unwrap(),
        direct
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["Spread", "Byte", "bytes"]
    );
    if defaulted {
        let mut ordered =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            ordered
                .invoke(support::id(&package, "Ordered"), vec![])
                .unwrap(),
            direct
        );
        assert_eq!(
            support::calls(&package, &ordered),
            vec!["Ordered", "Byte", "first", "mark", "second"]
        );
        let failure = vm.invoke(support::id(&package, "WrongStructured"), vec![]);
        assert!(failure.is_err(), "{failure:?}");
        assert_eq!(
            vm.invoke(support::id(&package, "Collected"), vec![])
                .unwrap(),
            direct
        );
        for entry in ["Structured", "StructuredFalse", "Matched"] {
            let mut structured =
                comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
                    .unwrap();
            assert_eq!(
                structured
                    .invoke(support::id(&package, entry), vec![])
                    .unwrap(),
                direct
            );
            assert_eq!(
                support::calls(&package, &structured),
                vec![entry, "Byte", "choose", "first", "mark", "second"]
            );
        }
    }
    for entry in ["Overflow", "Wrong", "WrongSpread"] {
        assert!(
            vm.invoke(support::id(&package, entry), vec![]).is_err(),
            "{entry}"
        );
        assert_eq!(
            vm.invoke(support::id(&package, "Collected"), vec![])
                .unwrap(),
            direct
        );
    }
}
