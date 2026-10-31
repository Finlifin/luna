// Supply the shipped module's primitive dependency without moving its declarations.
concat!(
    "mod fixture_tuple_primitives {\n",
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n} use fixture_tuple_primitives as primitives;\n",
    include_str!("../../../../library/builtin/types/tuple.fl"),
)
