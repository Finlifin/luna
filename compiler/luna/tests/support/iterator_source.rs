// Keep standalone protocol fixtures on the same primitive bindings as sysroot.
format!(
    "mod primitives {{ {} }} {}",
    include_str!("../../../../library/builtin/primitives.fl"),
    include_str!("../../../../library/builtin/protocols/iterator.fl"),
)
