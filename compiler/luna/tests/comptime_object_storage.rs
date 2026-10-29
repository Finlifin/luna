//! Comptime value receivers preserve snapshots without runtime Copy evidence.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn invoke(source: &str) -> Result<Value, String> {
    let package = support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }}
            typealias Type = .Type'builtin;
            {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )?;
    luna::invocation::invoke(&package, "run", 1_000_000)
}

#[test]
fn object_value_receivers_preserve_nested_snapshots() {
    assert_eq!(
        invoke(
            "
        pure comptime fn Identity<T> -> Type { T }
        comptime fn run() -> bool {
            let absent: ?bool = null;
            let types: meta.List<Type> = [Identity<bool>, bool];
            var object = {flag: true, flag: false, absent: absent,
                nested: {flag: true}, types: types};
            let saved = object;
            object.set(1, true);
            let changed = object;
            object = {};
            object.length() == 0 and (changed.get<bool>(.flag) matches true?)
                and saved.length() == 5 and (saved.get<bool>(.flag) matches false?)
                and (saved.get<?bool>(.absent) match {v? => v matches null, null => false})
                and (saved.get<meta.Object>(.nested) match {
                    v? => v.get<bool>(.flag) matches true?, null => false})
                and (saved.get<meta.List<Type>>(.types) match {
                    v? => v[0] == Identity<bool>, null => false})
        }
    "
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_object_payloads_do_not_require_runtime_copy() {
    assert_eq!(
        invoke(
            "
        struct Item { flag: bool }
        comptime fn run() -> bool {
            var object = {item: Item {flag: true}};
            let saved = object;
            object.set(0, Item {flag: false});
            (saved.get<Item>(.item) match {v? => v.flag, null => false})
                and (object.get<Item>(.item) match {v? => not v.flag, null => false})
        }
    "
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inactive_nominal_domains_are_valid_comptime_storage() {
    assert_eq!(
        invoke(
            "
        struct Item { flag: bool }
        comptime fn run() -> bool {
            let absent: ?Item = null;
            let empty: meta.List<Item> = [];
            var object = {item: absent, nested: {items: empty}};
            let saved = object;
            object.set(0, Item {flag: true});
            object.set(1, empty);
            (saved.get<?Item>(.item) match {v? => v matches null, null => false})
                and (saved.get<meta.Object>(.nested) match {
                    v? => v.get<meta.List<Item>>(.items) match {
                        items? => items.length() == 0, null => false}, null => false})
                and (object.get<Item>(.item) match {v? => v.flag, null => false})
                and (object.get<meta.List<Item>>(.nested) match {
                    v? => v.length() == 0, null => false})
        }
    "
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn lists_of_nominal_and_object_payloads_preserve_snapshots_without_copy() {
    assert_eq!(
        invoke(
            "
        struct Item { flag: bool }
        struct Holder { items: meta.List<Item> }
        comptime fn run() -> bool {
            let holder = Holder {items: [Item {flag: true}]};
            let saved = holder.items;
            holder.items.append(Item {flag: false});
            holder.items.append_all(saved);
            var objects: meta.List<meta.Object> = [{item: Item {flag: true}}];
            let old_objects = objects;
            objects.append({item: Item {flag: false}});
            let absent: ?Item = null;
            var optional_items: meta.List<?Item> = [];
            optional_items.append(absent);
            saved.length() == 1 and saved[0].flag
                and holder.items.length() == 3 and not holder.items[1].flag
                and holder.items[2].flag and old_objects.length() == 1
                and objects.length() == 2
                and (objects[1].get<Item>(.item) match {v? => not v.flag, null => false})
                and (optional_items[0] matches null)
        }
    "
        )
        .unwrap(),
        Value::Bool(true)
    );
}
