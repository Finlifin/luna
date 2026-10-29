// Supply the shipped module's primitive dependency without moving its declarations.
concat!(
    "mod fixture_operators_primitives {\n",
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n} use fixture_operators_primitives as primitives;\n",
    include_str!("../../../../library/builtin/protocols/ops.fl"),
)
