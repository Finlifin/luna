//! Nonlinear postconditions must connect callee entry values to caller snapshots.
#[path = "support/execution.rs"]
mod support;

#[test]
fn nonlinear_call_results_preserve_entry_snapshots_across_parameter_writes() {
    for degree in [2, 3] {
        let product = vec!["value"; degree].join(" * ");
        for overwrite in [false, true] {
            let body = if overwrite {
                "let saved=power(value);value=0;saved"
            } else {
                "power(value)"
            };
            let source = format!(
                "typealias Byte=.u8'builtin;
                fn power(value:Byte)->result:Byte
                where requires value<=6, ensures result==({product})'old {{
                    value={product};value
                }}
                fn relay(value:Byte)->result:Byte
                where requires value<=6, ensures result==({product})'old {{{body}}}
                fn run()->Byte {{relay(3)}}"
            );
            support::compile_with_protocols(&source, &Default::default())
                .unwrap_or_else(|e| panic!("degree={degree}, overwrite={overwrite}: {e}"));
            // A different entry input cannot be identified with the supplied
            // argument merely because both satisfy the same numeric bounds.
            let wrong_input = source.replace("power(value)", "power(0)");
            let error =
                support::compile_with_protocols(&wrong_input, &Default::default()).unwrap_err();
            assert!(error.contains("postcondition is not proven"), "{error}");
        }
    }
}

#[test]
fn nonlinear_snapshot_relations_cross_aliases_fields_and_equal_inputs() {
    for degree in [2, 3] {
        let product = |name: &str| vec![name; degree].join(" * ");
        let input = product("value");
        let field = product("box.value");
        let source = format!(
            "typealias Byte=.u8'builtin;
            struct Box {{value:Byte}}
            fn power(value:Byte)->result:Byte
            where requires value<=6, ensures result==({input})'old {{value={input};value}}
            fn aliases(value:Byte)->result:Byte
            where requires value<=6, ensures result==({input})'old {{
                let first=value;let second=first;let saved=power(second);value=0;saved
            }}
            fn field(box:Box)->result:Byte
            where requires box.value<=6, ensures result==({field})'old {{
                let saved=aliases(box.value);box.value=0;saved
            }}
            fn wrapped(value:Byte)->result:Box
            where requires value<=6, ensures result.value==({input})'old {{
                Box {{value:aliases(value)}}
            }}
            fn unwrapped(value:Byte)->result:Byte
            where requires value<=6, ensures result==({input})'old {{wrapped(value).value}}
            fn related(value:Byte,middle:Byte,other:Byte)->result:Byte
            where requires value<=6, requires other<=6,
                requires value==middle and middle==other,
                ensures result==({input})'old {{power(other)}}
            fn run()->bool {{
                field(Box {{value:3}})=={expected} and unwrapped(3)=={expected}
                    and related(3,3,3)=={expected}
            }}",
            expected = 3_u32.pow(degree as u32),
        );
        let value = support::run_iir(&source, "run", vec![]);
        assert_eq!(value, comptime_interpretor::Value::Bool(true));
        for invalid in [
            source.replace(
                "value==middle and middle==other",
                "value<=middle and middle<=other",
            ),
            source.replace("let saved=power(second)", "value=0;let saved=power(value)"),
            source.replace(
                "let saved=aliases(box.value)",
                "box.value=0;let saved=aliases(box.value)",
            ),
        ] {
            let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
            assert!(
                error.contains("postcondition is not proven"),
                "degree={degree}: {error}"
            );
        }
    }
}
