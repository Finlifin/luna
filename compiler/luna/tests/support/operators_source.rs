// Supply the shipped module's primitive and attribute dependencies without moving its declarations.
concat!(
    "mod fixture_operators_primitives {\n",
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n} use fixture_operators_primitives as primitives;\n",
    "mod fixture_operators_attrs {\n",
    include!("attrs_source.rs"),
    "\n} use fixture_operators_attrs as attrs;\n",
    include_str!("../../../../library/builtin/protocols/ops.fl"),
)
