//! Pre-interned well-known symbols used throughout the compiler.
//!
//! These are canonical [`Symbol`](hir::common::Symbol) values for names
//! that the compiler needs to recognise without string comparisons.
//! Calling the accessor functions is essentially free after the first
//! invocation thanks to [`once_cell`]-style lazy initialization inside
//! [`Symbol::intern`].

use hir::define_symbols;

define_symbols! {
    sym_bool    => "bool",
    sym_char    => "char",
    sym_string  => "String",
    sym_u8      => "u8",
    sym_u16     => "u16",
    sym_u32     => "u32",
    sym_u64     => "u64",
    sym_u128    => "u128",
    sym_usize   => "usize",
    sym_i8      => "i8",
    sym_i16     => "i16",
    sym_i32     => "i32",
    sym_i64     => "i64",
    sym_i128    => "i128",
    sym_isize   => "isize",
    sym_integer => "Integer",
    sym_f16     => "f16",
    sym_f32     => "f32",
    sym_f64     => "f64",
    sym_real    => "Real",
    sym_str     => "str",
    sym_void    => "void",
    sym_no_return => "NoReturn",
    sym_type    => "type",
    sym_undefined => "undefined",
    sym_any     => "Any",
    sym_any_value => "any",
    sym_unit    => "Unit",

    sym_debug   => "Debug",
    sym_display => "Display",
    sym_clone   => "Clone",
    sym_copy    => "Copy",
    sym_drop    => "Drop",
    sym_finalize => "Finalize",
    sym_eq      => "Eq",
    sym_ord     => "Ord",
    sym_hash    => "Hash",
    sym_iterator => "Iterator",
    sym_into    => "Into",
    sym_from    => "From",
}
