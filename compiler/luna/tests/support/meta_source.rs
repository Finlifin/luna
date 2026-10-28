concat!(
    "mod integer {\n", include_str!("../../../../library/meta/integer.fl"), "\n}\n",
    "mod object {\n", include_str!("../../../../library/meta/object.fl"), "\n}\n",
    "mod list {\n", include_str!("../../../../library/meta/list.fl"), "\n}\n",
    "mod structs {\n", include_str!("../../../../library/meta/structs.fl"), "\n}\n",
    "mod enums {\n", include_str!("../../../../library/meta/enums.fl"), "\n}\n",
    include_str!("../../../../library/meta/lib.fl"),
)
