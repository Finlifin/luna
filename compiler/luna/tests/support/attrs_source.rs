// Assemble the shipped attribute module hierarchy for isolated source fixtures.
concat!(
    "mod calling_convention { mod constructors {\n",
    include_str!("../../../../library/attrs/calling_convention/constructors.fl"),
    "\n}} mod staging { mod phase {\n",
    include_str!("../../../../library/attrs/staging/phase.fl"),
    "\n}}\n",
    include_str!("../../../../library/attrs/lib.fl"),
)
