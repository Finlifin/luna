// These isolated fixtures exercise meta data/builders without a scalar VM registry.
// Keep the real package root and omit only the separately tested math facade.
concat!(
    "mod data {\n",
    "mod integer {\n", include_str!("../../../../library/meta/data/integer.fl"), "\n}\n",
    "mod object {\n", include_str!("../../../../library/meta/data/object.fl"), "\n}\n",
    "mod list {\n", include_str!("../../../../library/meta/data/list.fl"), "\n}\n",
    "} mod definitions {\n",
    "mod structs {\n", include_str!("../../../../library/meta/definitions/structs.fl"), "\n}\n",
    "mod enums {\n", include_str!("../../../../library/meta/definitions/enums.fl"), "\n}\n",
    "}\n",
    include_str!("../../../../library/meta/lib.fl"),
).replace("use numeric.scalar_math;\n", "").replace("pub mod math {\n    pub use scalar_math.*;\n}\n\n", "")
