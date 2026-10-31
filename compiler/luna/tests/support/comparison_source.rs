// Isolated fixtures reuse the real centralized primitive bindings.
concat!(
    "mod primitives {\n",
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n}\n",
    include_str!("../../../../library/builtin/numeric/comparison.fl"),
    "\nmod builtin {pub use primitives.*;}\n",
    "mod comptime_comparison {\n",
    include_str!("../../../../library/meta/numeric/equality.fl"),
    "\n} pub use comptime_comparison.ComptimeEq;\n",
)
