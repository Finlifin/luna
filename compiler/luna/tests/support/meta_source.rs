// These isolated fixtures exercise meta data/builders without a scalar VM registry.
// Keep the real package root and omit only the separately tested math facade.
// Embed the real primitive bindings for this single-package fixture; production
// resolves the same names through the builtin package dependency.
concat!(
    "mod attrs {\n", include!("attrs_source.rs"), "\n}\n",
    "mod primitives {\n", include_str!("../../../../library/builtin/primitives.fl"),
    "\n} mod comparison {\n", include_str!("../../../../library/builtin/numeric/comparison.fl"), "\n} mod builtin {pub use primitives.*; pub use comparison.Order;\nmod bindings {\n", include_str!("../../../../library/builtin/bindings.fl"), "\n} pub mod compiler {pub use bindings.*;} }\n",
    "mod bindings {\n", include_str!("../../../../library/meta/bindings.fl"), "\n}\n",
    "mod text_impl {mod bindings {\n", include_str!("../../../../library/meta/text_impl/bindings.fl"), "\n}}\n",
    "mod types {mod equality {\n", include_str!("../../../../library/meta/types/equality.fl"), "\n}}\n",
    "mod numeric {mod bindings {\n", include_str!("../../../../library/meta/numeric/bindings.fl"), "\n} mod ordering {\n", include_str!("../../../../library/meta/numeric/ordering.fl"), "\n} mod equality {\n", include_str!("../../../../library/meta/numeric/equality.fl"), "\n}}\n",
    "mod data {\n",
    "mod hashing {\n", include_str!("../../../../library/meta/data/hashing.fl"), "\n}\n",
    "mod patterns {\n", include_str!("../../../../library/meta/data/patterns.fl"), "\n}\n",
    "mod integer {\n", include_str!("../../../../library/meta/data/integer.fl"), "\n}\n",
    "mod object {\n", include_str!("../../../../library/meta/data/object.fl"), "\n}\n",
    "mod list {\n", include_str!("../../../../library/meta/data/list.fl"), "\n}\n",
    "mod list_equality {\n", include_str!("../../../../library/meta/data/list_equality.fl"), "\n}\n",
    "mod list_hashing {\n", include_str!("../../../../library/meta/data/list_hashing.fl"), "\n}\n",
    "mod list_ordering {\n", include_str!("../../../../library/meta/data/list_ordering.fl"), "\n}\n",
    "mod text {\n", include_str!("../../../../library/meta/data/text.fl"), "\n}\n",
    "mod optional {\n", include_str!("../../../../library/meta/data/optional.fl"), "\n}\n",
    "mod errors {\n", include_str!("../../../../library/meta/data/errors.fl"), "\n}\n",
    "} mod reflection {mod structure {\n",
    include_str!("../../../../library/meta/reflection/structure.fl"), "\n}}\n",
    "mod definitions {\n",
    "mod fields {\n", include_str!("../../../../library/meta/definitions/fields.fl"), "\n}\n",
    "mod structs {\n", include_str!("../../../../library/meta/definitions/structs.fl"), "\n}\n",
    "mod enums {\n", include_str!("../../../../library/meta/definitions/enums.fl"), "\n}\n",
    "mod newtypes {\n", include_str!("../../../../library/meta/definitions/newtypes.fl"), "\n}\n",
    "}\n",
    include_str!("../../../../library/meta/lib.fl"),
).replace("use numeric.{scalar_math, equality, ordering};\n", "use numeric.{equality, ordering};\n").replace("pub mod math {\n    pub use scalar_math.*;\n}\n\n", "")
