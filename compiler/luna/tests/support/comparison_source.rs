// Isolated fixtures reuse the real centralized primitive bindings.
concat!(
    "mod primitives {\n",
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n}\n",
    include_str!("../../../../library/builtin/numeric/comparison.fl"),
)
