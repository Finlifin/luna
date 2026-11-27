//! Bind the real direct-passing attribute in single-package effect fixtures.

macro_rules! effect_fixture {
    ($source:literal) => {
        concat!(
            $source,
            "\nmod bindings { pub typealias Object = .Object'builtin; pub typealias Symbol = .Symbol'builtin; }",
            "\nmod attrs { pub mod effects {\n",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../library/attrs/effects_impl/strategy.fl"
            )),
            "\n} }\n"
        )
    };
    (format: $($arguments:tt)*) => {{
        let mut source = format!($($arguments)*);
        source.push_str(effect_fixture!(""));
        source
    }};
}
