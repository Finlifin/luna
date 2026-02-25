//! Pre-interned well-known symbols used throughout the compiler.
//!
//! These are canonical [`Symbol`](hir::common::Symbol) values for names
//! that the compiler needs to recognise without string comparisons.
//! Calling the accessor functions is essentially free after the first
//! invocation thanks to [`once_cell`]-style lazy initialization inside
//! [`Symbol::intern`].

use hir::common::Symbol;

// ── Macro helper ─────────────────────────────────────────────────────────────

macro_rules! define_symbols {
    ( $( $(#[$meta:meta])* $name:ident => $str:expr ),* $(,)? ) => {
        $(
            $(#[$meta])*
            #[inline]
            pub fn $name() -> Symbol { Symbol::intern($str) }
        )*

        /// Return every well-known symbol (useful for bulk pre-interning).
        pub fn all() -> &'static [(&'static str, fn() -> Symbol)] {
            &[
                $( ($str, $name as fn() -> Symbol), )*
            ]
        }
    };
}

// ── Symbol catalogue ─────────────────────────────────────────────────────────

define_symbols! {
    // ── Primitive type names ─────────────────────────────────────────────
    /// `Int`
    sym_int     => "Int",
    /// `Float`
    sym_float   => "Float",
    /// `Bool`
    sym_bool    => "Bool",
    /// `Char`
    sym_char    => "Char",
    /// `String`
    sym_string  => "String",
    /// `Unit` (the `()` type written as a name)
    sym_unit    => "Unit",
    /// `Never` (the `!` type written as a name)
    sym_never   => "Never",

    // ── Built-in function names ──────────────────────────────────────────
    /// `print`
    sym_print   => "print",
    /// `println`
    sym_println => "println",
    /// `assert`
    sym_assert  => "assert",
    /// `panic`
    sym_panic   => "panic",
    /// `todo`
    sym_todo    => "todo",
    /// `unreachable`
    sym_unreachable => "unreachable",
    /// `sizeof`
    sym_sizeof  => "sizeof",
    /// `typeof`
    sym_typeof  => "typeof",

    // ── Well-known trait / protocol names ─────────────────────────────────
    /// `Debug`
    sym_debug   => "Debug",
    /// `Display`
    sym_display => "Display",
    /// `Clone`
    sym_clone   => "Clone",
    /// `Copy`
    sym_copy    => "Copy",
    /// `Drop`
    sym_drop    => "Drop",
    /// `Eq`
    sym_eq      => "Eq",
    /// `Ord`
    sym_ord     => "Ord",
    /// `Hash`
    sym_hash    => "Hash",
    /// `Iterator`
    sym_iterator => "Iterator",
    /// `Into`
    sym_into    => "Into",
    /// `From`
    sym_from    => "From",

    // ── Operator method names ────────────────────────────────────────────
    /// `add` – binary `+`
    sym_add     => "add",
    /// `sub` – binary `-`
    sym_sub     => "sub",
    /// `mul` – binary `*`
    sym_mul     => "mul",
    /// `div` – binary `/`
    sym_div     => "div",
    /// `rem` – binary `%`
    sym_rem     => "rem",
    /// `neg` – unary `-`
    sym_neg     => "neg",
    /// `not` – unary `!`
    sym_not     => "not",

    // ── Other well-known names ───────────────────────────────────────────
    /// `main` – program entry point
    sym_main    => "main",
    /// `self`
    sym_self_lower => "self",
    /// `Self`
    sym_self_upper => "Self",
    /// `super`
    sym_super   => "super",
    /// `crate`
    sym_crate   => "crate",
}
