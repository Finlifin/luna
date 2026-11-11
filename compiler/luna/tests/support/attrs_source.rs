// Assemble the shipped attributes and their primitive/domain dependencies.
// Full sysroot cases separately cover meta's operations and package wiring.
concat!(
    "mod builtin {\n",
    include_str!("../../../../library/builtin/primitives.fl"),
    "\n} mod meta {\n",
    include_str!("../../../../library/meta/types/bindings.fl"),
    "\n} mod bindings {\n",
    include_str!("../../../../library/attrs/bindings.fl"),
    "\n}\n",
    "mod calling_convention { mod constructors {\n",
    include_str!("../../../../library/attrs/calling_convention/constructors.fl"),
    "\n}} mod staging { mod phase {\n",
    include_str!("../../../../library/attrs/staging/phase.fl"),
    "\n}}\n",
    include_str!("../../../../library/attrs/lib.fl"),
)
