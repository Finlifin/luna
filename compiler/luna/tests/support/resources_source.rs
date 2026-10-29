// Supply the shipped module's primitive dependency without moving its declarations.
concat!(
    "mod fixture_resources_primitives {\n",
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n} use fixture_resources_primitives as primitives;\n",
    include_str!("../../../../library/builtin/protocols/resources.fl"),
)
