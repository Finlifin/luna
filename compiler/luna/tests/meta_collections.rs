//! End-to-end comptime execution tests for `meta.Map`, `meta.MapEntry`, and `meta.Set`
//! against the real built-in packages (`--sysroot library`).
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn meta_map_supports_crud_order_snapshots_types_equality_and_hashing() {
    let source = r#"
        pure comptime fn Identity<T> -> Type { T }

        struct CustomKey {
            bucket: Integer,
            label: Symbol,
        }

        comptime impl meta.ComptimeEq for CustomKey {
            comptime fn eq(self, other: Self) -> bool {
                self.bucket == other.bucket and self.label == other.label
            }
        }

        comptime impl meta.ComptimeHash for CustomKey {
            comptime fn hash(self) -> u64 {
                self.bucket.as(meta.ComptimeHash).hash()
            }
        }

        comptime fn run() -> bool {
            -- 1. Map<Symbol, Integer> via default meta.prelude: CRUD, order, snapshots.
            var counts = Map<Symbol, Integer>.new();
            if not counts.is_empty() or counts.length() != 0 {
                return false;
            }
            if counts.insert(.alpha, 10).is_some() {
                return false;
            }
            if counts.insert(.beta, 20).is_some() {
                return false;
            }
            if counts.insert(.gamma, 30).is_some() {
                return false;
            }
            let replaced = counts.insert(.beta, 200) match {
                v? => v,
                null => return false,
            };
            if replaced != 20 or counts.length() != 3 {
                return false;
            }
            let beta_now = counts.get(.beta) match {
                v? => v,
                null => return false,
            };
            if beta_now != 200 or not counts.contains_key(.alpha) or counts.contains_key(.missing) {
                return false;
            }

            let expected_keys: List<Symbol> = [.alpha, .beta, .gamma];
            let expected_vals: List<Integer> = [10, 200, 30];
            if not counts.keys().as(ComptimeEq).eq(expected_keys) {
                return false;
            }
            if not counts.values().as(ComptimeEq).eq(expected_vals) {
                return false;
            }
            let entry_list = counts.entries();
            if entry_list.length() != 3
                or not entry_list[1].as(ComptimeEq).eq(MapEntry<Symbol, Integer> { key: .beta, value: 200 })
            {
                return false;
            }

            let snap = counts;
            let removed = counts.remove(.beta) match {
                v? => v,
                null => return false,
            };
            if removed != 200
                or counts.remove(.beta).is_some()
                or counts.length() != 2
                or snap.length() != 3
            {
                return false;
            }
            let after_remove_keys: List<Symbol> = [.alpha, .gamma];
            if not counts.keys().as(ComptimeEq).eq(after_remove_keys) {
                return false;
            }

            -- Self-insert_all and non-mutating merged.
            var extra = Map<Symbol, Integer>.new();
            extra.insert(.gamma, 333);
            extra.insert(.delta, 40);
            let combined = counts.merged(extra);
            if counts.length() != 2 or extra.length() != 2 or combined.length() != 3 {
                return false;
            }
            let combined_gamma = combined.get(.gamma) match {
                v? => v,
                null => return false,
            };
            if combined_gamma != 333 {
                return false;
            }
            counts.insert_all(counts);
            counts.insert_all(extra);
            if not counts.as(ComptimeEq).eq(combined) {
                return false;
            }

            -- 2. Order-independent ComptimeEq & ComptimeHash on Map<str, List<Integer>>.
            var m1 = Map<str, List<Integer>>.new();
            m1.insert("one", [1, 2]);
            m1.insert("two", [3, 4]);
            var m2 = Map<str, List<Integer>>.new();
            m2.insert("two", [3, 4]);
            m2.insert("one", [1, 2]);
            if not m1.as(ComptimeEq).eq(m2) {
                return false;
            }
            if m1.as(ComptimeHash).hash() != m2.as(ComptimeHash).hash() {
                return false;
            }
            m2.insert("one", [1, 9]);
            if m1.as(ComptimeEq).eq(m2) {
                return false;
            }

            m1.clear();
            if not m1.is_empty() or m1.length() != 0 {
                return false;
            }

            -- 3. Type keys (ComptimeEq without ComptimeHash): provably distinct types & normal-form identity.
            var types = Map<Type, str>.new();
            types.insert(bool, "primitive-bool");
            types.insert(u32, "primitive-u32");
            types.insert(CustomKey, "nominal-custom");
            if types.length() != 3 or types.contains_key(str) {
                return false;
            }
            let p_label = types.get(bool) match {
                s? => s,
                null => return false,
            };
            let c_label = types.get(CustomKey) match {
                s? => s,
                null => return false,
            };
            if p_label != "primitive-bool" or c_label != "nominal-custom" {
                return false;
            }

            var nf_types = Map<Type, str>.new();
            nf_types.insert(Identity<bool>, "nf-identity-bool");
            let nf_label = nf_types.get(Identity<bool>) match {
                s? => s,
                null => return false,
            };
            if nf_types.length() != 1 or nf_label != "nf-identity-bool" {
                return false;
            }

            -- 4. Custom user struct keys with ComptimeEq & ComptimeHash.
            var custom = Map<CustomKey, Integer>.new();
            let k1 = CustomKey { bucket: 7, label: .x };
            let k2 = CustomKey { bucket: 7, label: .y };
            custom.insert(k1, 100);
            custom.insert(k2, 200);
            var custom_rev = Map<CustomKey, Integer>.new();
            custom_rev.insert(CustomKey { bucket: 7, label: .y }, 200);
            custom_rev.insert(CustomKey { bucket: 7, label: .x }, 100);
            custom.as(ComptimeEq).eq(custom_rev)
                and custom.as(ComptimeHash).hash() == custom_rev.as(ComptimeHash).hash()
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}

#[test]
fn meta_set_and_nested_collections_work_in_comptime() {
    let source = r#"
        pure comptime fn Boxed<T> -> Type { T }

        comptime fn run() -> bool {
            -- 1. Set<Integer> from_list, insert/remove, algebra, order-independent eq/hash.
            var left = Set<Integer>.from_list([1, 2, 2, 3, 1]);
            let right = Set<Integer>.from_list([3, 4, 2]);
            let reordered = Set<Integer>.from_list([3, 1, 2]);
            if left.length() != 3 or not left.contains(2) or left.contains(99) {
                return false;
            }
            let expected_order: List<Integer> = [1, 2, 3];
            if not left.to_list().as(ComptimeEq).eq(expected_order) {
                return false;
            }
            if not left.as(ComptimeEq).eq(reordered)
                or left.as(ComptimeHash).hash() != reordered.as(ComptimeHash).hash()
            {
                return false;
            }

            let joined = left.union_with(right);
            let shared = left.intersection(right);
            let only_left = left.difference(right);
            if not joined.as(ComptimeEq).eq(Set<Integer>.from_list([1, 2, 3, 4])) {
                return false;
            }
            if not shared.as(ComptimeEq).eq(Set<Integer>.from_list([2, 3])) {
                return false;
            }
            if not only_left.as(ComptimeEq).eq(Set<Integer>.from_list([1])) {
                return false;
            }
            if not shared.is_subset(left)
                or not left.is_superset(shared)
                or not only_left.is_disjoint(right)
                or left.is_disjoint(right)
            {
                return false;
            }

            if left.insert(2) or not left.insert(9) or left.length() != 4 {
                return false;
            }
            if not left.remove(2) or left.remove(2) or left.length() != 3 {
                return false;
            }
            left.clear();
            if not left.is_empty() or left.length() != 0 {
                return false;
            }

            -- 2. Set<Symbol> and Set<Type> (preserving normal-form identity).
            var tags = Set<Symbol>.new();
            tags.insert(.read);
            tags.insert(.write);
            if tags.length() != 2 or not tags.contains(.read) {
                return false;
            }

            let type_set = Set<Type>.from_list([u32, bool, u32]);
            let nf_set = Set<Type>.from_list([Boxed<u32>, Boxed<u32>]);
            if type_set.length() != 2
                or not type_set.contains(u32)
                or not type_set.contains(bool)
                or type_set.contains(str)
                or nf_set.length() != 1
                or not nf_set.contains(Boxed<u32>)
            {
                return false;
            }

            -- 3. Nested Map<Symbol, Set<Integer>> with ComptimeEq & ComptimeHash.
            var g1 = Map<Symbol, Set<Integer>>.new();
            g1.insert(.even, Set<Integer>.from_list([2, 4, 6]));
            g1.insert(.odd, Set<Integer>.from_list([1, 3]));

            var g2 = Map<Symbol, Set<Integer>>.new();
            g2.insert(.odd, Set<Integer>.from_list([3, 1]));
            g2.insert(.even, Set<Integer>.from_list([6, 4, 2]));

            g1.as(ComptimeEq).eq(g2) and g1.as(ComptimeHash).hash() == g2.as(ComptimeHash).hash()
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}
