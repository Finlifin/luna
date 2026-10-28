// Isolated fixtures reuse the real centralized primitive bindings.
concat!(
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n",
    include_str!("../../../../library/builtin/numeric/comparison.fl"),
)
