// Isolated fixtures use the shipped phase-specific traits and compiler bindings.
concat!(
    "mod primitives {\n", include_str!("../../../../library/builtin/primitives.fl"), "\n}\nuse primitives.*;\n",
    "mod builtin {pub use primitives.*;}\n",
    "mod text {\n", include_str!("../../../../library/builtin/text_impl/bytes.fl"), "\n}\nuse text.TextBytes;\n",
    "mod meta {mod text_impl {mod bindings {\n",
    include_str!("../../../../library/meta/text_impl/bindings.fl"),
    "\n}} mod text {\n", include_str!("../../../../library/meta/data/text.fl"),
    "\n} pub use text.ComptimeTextBytes;}\nuse meta.ComptimeTextBytes;\n",
)
