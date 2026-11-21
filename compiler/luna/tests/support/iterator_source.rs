// Keep standalone protocol fixtures on the same sibling modules as sysroot.
format!(
    "pub mod primitives {{\n{}\n}}\nuse primitives.*;\npub mod bindings {{\n{}\n}}\npub mod meta {{\n    use primitives.Type;\n    pub const List: pure comptime fn<T: Type> -> Type = .List'builtin;\n}}\npub mod types {{\n    pub mod tuple {{\n{}\n    }}\n}}\npub mod numeric {{\n    pub mod comparison {{\n{}\n    }}\n}}\npub mod mem_impl {{\n    pub mod raw {{\n        use bindings.{{try_u128_to_usize, usize_representation}}\n        use primitives.{{u128, usize}}\n        pub fn checked_add_usize(a: usize, b: usize) -> ?usize {{\n            let raw_a = usize_representation(a);\n            let raw_b = usize_representation(b);\n            let u128_max: u128 = 340282366920938463463374607431768211455;\n            if raw_a > u128_max - raw_b {{\n                return null;\n            }}\n            try_u128_to_usize(raw_a + raw_b)\n        }}\n    }}\n}}\npub mod protocols {{\n    pub mod resources {{\n{}\n    }}\n}}\npub use protocols.resources.{{Clone, Copy, Drop, Once}};\n{}",
    include_str!("../../../../library/builtin/primitives.fl"),
    include_str!("../../../../library/builtin/bindings.fl"),
    include_str!("../../../../library/builtin/types/tuple.fl"),
    include_str!("../../../../library/builtin/numeric/comparison.fl"),
    include_str!("../../../../library/builtin/protocols/resources.fl"),
    include_str!("../../../../library/builtin/protocols/iterator.fl"),
)
