#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn tuple_indexed_handles_heterogeneous_empty_guards_and_once_elements() {
    let source = r#"
        struct Ticket {
            id: u32,
        }

        impl builtin.Drop for Ticket {
            fn drop(*self) {}
        }

        fn sum_mixed(pair: Tuple<u32, bool, str>, early: bool) -> u32 {
            var acc: u32 = 0;
            inline for (i, val) if i != 1 in pair'tuple_indexed {
                const slot: u32 = if i == 0 { 10 } else { 300 };
                inline if val'type == u32 {
                    let v: u32 = val;
                    assumes acc < 10000;
                    assumes v < 1000;
                    acc = acc + slot + v;
                } else {
                    assumes acc < 10000;
                    acc = acc + slot;
                }
                inline if i == 0 {
                    if early {
                        break;
                    }
                }
            }
            acc
        }

        fn inspect_empty(empty: Tuple<>) -> u32 {
            var seen: u32 = 0;
            inline for (i, _) in empty'tuple_indexed {
                const base: u32 = if i == 0 { 1 } else { 2 };
                seen = base + 1;
            }
            seen
        }

        fn nested_tuples(outer: Tuple<Tuple<u32, u32>, u32>) -> u32 {
            var total: u32 = 0;
            inline for (i, item) in outer'tuple_indexed {
                const factor: u32 = if i == 0 { 1 } else { 10 };
                inline when {
                    item'type == Tuple<u32, u32> => {
                        inline for (j, inner) in item'tuple_indexed {
                            const shift: u32 = if j == 0 { 1 } else { 2 };
                            let v: u32 = inner;
                            assumes total < 100000;
                            assumes v < 1000;
                            total = total + v * shift * factor;
                        }
                    },
                    item'type == u32 => {
                        let n: u32 = item;
                        assumes total < 100000;
                        assumes n < 1000;
                        total = total + n * factor;
                    },
                }
            }
            total
        }

        fn borrow_once(items: Tuple<Ticket, Ticket>) -> u32 {
            var acc: u32 = 0;
            inline for (i, item) in items'tuple_indexed {
                const bump: u32 = if i == 0 { 1 } else { 10 };
                let id = item.ref.id.*;
                assumes acc < 100000;
                assumes id < 1000;
                acc = acc + id * bump;
                inline if i == 0 {
                    if id > 50 {
                        break;
                    }
                }
            }
            acc
        }

        fn run() -> bool {
            let full = sum_mixed((5, true, "hi"), false);
            let brk = sum_mixed((5, true, "hi"), true);
            let emp = inspect_empty(());
            let nest = nested_tuples(((3, 4), 2));
            let once_all = borrow_once((Ticket { id: 7 }, Ticket { id: 8 }));
            let once_early = borrow_once((Ticket { id: 60 }, Ticket { id: 9 }));
            full == 315 and brk == 15 and emp == 0 and nest == 31 and once_all == 87 and once_early == 60
        }
    "#;

    let package = support::compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn tuple_indexed_rejects_invalid_patterns_and_non_tuples() {
    for bad in [
        "fn run() { inline for x in (true, false)'tuple_indexed {} }",
        "fn run() { inline for (i, i) in (true, false)'tuple_indexed {} }",
        "fn run() { inline for (i, x) in 42u32'tuple_indexed {} }",
        "fn run() { let v = (true, false)'tuple_indexed; }",
    ] {
        assert!(
            support::compile(bad).is_err(),
            "expected failure for: {bad}"
        );
    }
    let purity_failure = support::compile(
        "pure fn bad(comptime enabled: bool, values: Tuple<bool, bool>, pointer: *bool) -> bool {
            inline for (i, elem) if enabled in values'tuple_indexed {
                inline if i == 0 { pointer.* = elem; }
            }
            true
        }
        fn run(pointer: *bool) -> bool { bad(false, (true, false), pointer) }",
    )
    .unwrap_err();
    assert!(
        purity_failure.contains("P4") || purity_failure.contains("pure"),
        "{purity_failure}"
    );
}

#[test]
fn staged_two_phase_proto_display_and_comptime_format() {
    let source = r#"
        enum NoErr { never }

        trait ProtoWriter {
            assoc Err: Type;
            fn push_byte(*self, b: u8) -> !Err void;
            fn push_str(*self, s: str) -> !Err void {
                let n = s.byte_length();
                var idx: u64 = 0;
                while idx < n {
                    let b = s.byte(idx) match {
                        v? => v,
                        null => break,
                    }
                    self.push_byte(b)!;
                    idx = idx + 1;
                }
            }
        }

        struct CounterWriter {
            bytes: u32,
            last: u8,
        }

        impl ProtoWriter for CounterWriter {
            assoc Err: Type = NoErr;
            fn push_byte(*self, b: u8) -> !NoErr void {
                assumes self.bytes.* < 1000000;
                self.bytes.* = self.bytes.* + 1;
                self.last.* = b;
            }
        }

        trait ProtoDisplay {
            fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ProtoWriter).Err void
            where
                W :- ProtoWriter;

            ^attrs.comptime_phase
            comptime fn comptime_fmt(self, .format: str = "") -> str;
        }

        struct IntFormatOpts {
            is_hex: bool,
        }

        impl builtin.Splice for IntFormatOpts {
            assoc RuntimeTarget: Type = IntFormatOpts;
            fn splice(comptime data: IntFormatOpts) -> IntFormatOpts {
                IntFormatOpts { is_hex: data.is_hex }
            }
        }

        pure comptime fn parse_int_opts(format: str) -> IntFormatOpts {
            if format == "" {
                IntFormatOpts { is_hex: false }
            } else if format == "format: .hex" {
                IntFormatOpts { is_hex: true }
            } else {
                assumes false;
                IntFormatOpts { is_hex: false }
            }
        }

        pure comptime fn parse_str_opts(format: str) -> bool {
            if format == "" {
                true
            } else {
                assumes false;
                false
            }
        }

        impl ProtoDisplay for u32 {
            fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ProtoWriter).Err void
            where
                W :- ProtoWriter
            {
                let opts: IntFormatOpts = parse_int_opts(format);
                if opts.is_hex {
                    output.push_str("0xff")
                } else {
                    output.push_str("255")
                }
            }
        }

        comptime impl ProtoDisplay for u32 {
            comptime fn comptime_fmt(self, .format: str = "") -> str {
                let opts: IntFormatOpts = parse_int_opts(format);
                if opts.is_hex { "0xff" } else { "255" }
            }
        }

        impl ProtoDisplay for str {
            fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ProtoWriter).Err void
            where
                W :- ProtoWriter
            {
                let ok: bool = parse_str_opts(format);
                if ok {
                    output.push_str(self.*)
                }
            }
        }

        comptime impl ProtoDisplay for str {
            comptime fn comptime_fmt(self, .format: str = "") -> str {
                let ok: bool = parse_str_opts(format);
                if ok { self } else { "" }
            }
        }

        comptime impl ProtoDisplay for Integer {
            comptime fn comptime_fmt(self, .format: str = "") -> str {
                let opts: IntFormatOpts = parse_int_opts(format);
                if opts.is_hex { "0x2a" } else { "42" }
            }
        }

        struct OnceTicket {
            tag: u32,
        }

        impl builtin.Drop for OnceTicket {
            fn drop(*self) {}
        }

        impl ProtoDisplay for OnceTicket {
            fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ProtoWriter).Err void
            where
                W :- ProtoWriter
            {
                let ok: bool = parse_str_opts(format);
                if ok {
                    output.push_str("ticket")
                }
            }
        }

        typealias AnyProto = refines { Itself :- ProtoDisplay } Any;

        pure comptime fn PickProto(format: str) -> R: Type
        where
            ensures Type.predicates.is_tuple(R),
        {
            if format == "hex_and_plain" or format == "plain_two" {
                Tuple<.Es = [AnyProto, AnyProto]>
            } else {
                Tuple<>
            }
        }

        pure comptime fn specs_for(format: str) -> meta.List<str> {
            if format == "hex_and_plain" {
                ["format: .hex", ""]
            } else if format == "plain_two" {
                ["", ""]
            } else {
                []
            }
        }

        pure comptime fn spec_at(specs: meta.List<str>, idx: Integer) -> str {
            specs.get(idx) match {
                s? => s,
                null => "",
            }
        }

        fn write_proto(
            output: *W,
            comptime format: str,
            ...values: defer PickProto(format),
        ) -> !W.as(ProtoWriter).Err void
        where
            W :- ProtoWriter
        {
            const specs: meta.List<str> = specs_for(format);
            inline for (i, value) in values'tuple_indexed {
                const field_fmt: str = spec_at(specs, i);
                const T = value'type;
                inline if T :- ProtoDisplay {
                    value.ref.as(ProtoDisplay).fmt(output, .format = field_fmt)!;
                }
            }
        }

        comptime fn comptime_proto_format(
            comptime format: str,
            ...values: defer PickProto(format),
        ) -> str {
            const specs: meta.List<str> = specs_for(format);
            var out = "";
            inline for (i, value) in values'tuple_indexed {
                const field_fmt: str = spec_at(specs, i);
                const T = value'type;
                inline if T :- ProtoDisplay {
                    out = out.concat(value.as(ProtoDisplay).comptime_fmt(.format = field_fmt));
                }
            }
            out
        }
    "#;

    let valid = format!(
        r#"
        {source}

        fn run() -> bool {{
            var w1 = CounterWriter {{ bytes: 0, last: 0 }};
            let n: u32 = 255;
            let r1 = write_proto(w1.ref, "hex_and_plain", n, "hello") match {{
                _! => w1.bytes == 9 and w1.last == 104 + 7,
                error _ => false,
            }};

            var w2 = CounterWriter {{ bytes: 0, last: 0 }};
            let t = OnceTicket {{ tag: 1 }};
            let r2 = write_proto(w2.ref, "hex_and_plain", n, t) match {{
                _! => w2.bytes == 10,
                error _ => false,
            }};
            r1 and r2
        }}

        comptime fn run_ct() -> bool {{
            let a: u32 = 255;
            let b: Integer = 42;
            let s1 = comptime_proto_format("hex_and_plain", a, "world");
            let s2 = comptime_proto_format("hex_and_plain", b, a);
            s1 == "0xffworld" and s2 == "0x2a255"
        }}
        "#
    );

    let package = support::compile(&valid).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );

    let ct_res = support::invoke(&valid, "run_ct").unwrap();
    assert_eq!(ct_res, Value::Bool(true));

    // Invalid option for slot 0's concrete type (`str` given `"format: .hex"`) must be rejected
    let invalid = format!(
        r#"
        {source}
        fn run() -> bool {{
            var w = CounterWriter {{ bytes: 0, last: 0 }};
            write_proto(w.ref, "hex_and_plain", "bad_str", "ok") match {{
                _! => true,
                error _ => false,
            }}
        }}
        "#
    );
    let err = support::compile(&invalid)
        .and_then(|pkg| luna::instantiation::instantiate(&pkg, "run", 1_000_000).map(|_| ()));
    assert!(
        err.is_err(),
        "invalid format property on `str` must be rejected at compile time"
    );
}
