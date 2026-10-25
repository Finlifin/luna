use comptime_runtime::Budget;
use comptime_runtime::object::{Entry, MergeError, Object, merge_named_attributes};

fn properties(entries: &[(&str, i32)]) -> Object<i32> {
    Object::from_entries(
        entries
            .iter()
            .map(|(key, value)| Entry::Property {
                key: (*key).into(),
                value: *value,
            })
            .collect(),
    )
}

#[test]
fn object_preserves_mixed_entries_duplicates_and_nested_values() {
    let entries = vec![
        Entry::Value(1),
        Entry::Property {
            key: "x".into(),
            value: 2,
        },
        Entry::Value(3),
        Entry::Property {
            key: "x".into(),
            value: 4,
        },
    ];
    let object = Object::from_entries(entries.clone());
    assert_eq!(object.entries(), entries.as_slice());
    assert_eq!(object.clone().into_entries(), entries);
    let nested = Object::from_entries(vec![Entry::Property {
        key: "nested".into(),
        value: object,
    }]);
    assert_eq!(
        nested.to_string(),
        "(object (property (key \"nested\") (value (object (value 1) (property (key \"x\") (value 2)) (value 3) (property (key \"x\") (value 4))))))"
    );
}

#[test]
fn configurations_precede_inner_then_outer_prefixes() {
    let outer = properties(&[("shared", 1), ("outer", 10)]);
    let inner = properties(&[("shared", 2), ("inner", 20), ("shared", 3)]);
    let config = [("shared", 99), ("config", 30)];
    let values = merge_named_attributes(
        config.iter().map(|(k, v)| (*k, v)),
        &[&outer, &inner],
        4,
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(values.get("shared"), Some(&1));
    assert_eq!(
        values.iter().map(|(k, v)| (k, *v)).collect::<Vec<_>>(),
        vec![("config", 30), ("inner", 20), ("outer", 10), ("shared", 1)]
    );
    let inner_only = merge_named_attributes([], &[&inner], 2, &mut Budget::default()).unwrap();
    assert_eq!(inner_only.get("shared"), Some(&3));
    assert_eq!(values.get("absent"), None);
}

#[test]
fn merging_borrows_nonclone_payloads_and_keeps_input_identity() {
    struct Payload(u8);
    let object = Object::from_entries(vec![Entry::Property {
        key: "value".into(),
        value: Payload(42),
    }]);
    let Entry::Property { value, .. } = &object.entries()[0] else {
        unreachable!()
    };
    let merged = merge_named_attributes([], &[&object], 1, &mut Budget::default()).unwrap();
    assert_eq!(merged.get("value").unwrap().0, 42);
    assert!(std::ptr::eq(merged.get("value").unwrap(), value));
}

#[test]
fn unique_key_limit_allows_overrides_but_never_partial_results() {
    let override_only = properties(&[("x", 2), ("x", 3)]);
    let distinct = properties(&[("x", 1), ("y", 4)]);
    let original = distinct.clone();
    let result =
        merge_named_attributes([("x", &1)], &[&override_only], 1, &mut Budget::default()).unwrap();
    assert_eq!(result.get("x"), Some(&3));
    assert!(matches!(
        merge_named_attributes([], &[&distinct], 1, &mut Budget::default()),
        Err(MergeError::EntryLimit)
    ));
    assert_eq!(distinct, original);
    assert!(merge_named_attributes([], &[&distinct], 2, &mut Budget::default()).is_ok());
    assert!(matches!(
        merge_named_attributes([], &[&override_only], 0, &mut Budget::default()),
        Err(MergeError::EntryLimit)
    ));
}

#[test]
fn every_insufficient_budget_rejects_without_modifying_inputs() {
    let object = properties(&[("long_key", 1), ("another", 2), ("long_key", 3)]);
    let original = object.clone();
    let mut full = Budget::default();
    let start = full.steps;
    merge_named_attributes([], &[&object], 2, &mut full).unwrap();
    let required = start - full.steps;
    for steps in 0..required {
        let mut budget = Budget {
            steps,
            ..Budget::default()
        };
        assert!(
            matches!(
                merge_named_attributes([], &[&object], 2, &mut budget),
                Err(MergeError::BudgetExhausted)
            ),
            "{steps}"
        );
        assert_eq!(object, original);
    }
    let mut exact = Budget {
        steps: required,
        ..Budget::default()
    };
    assert_eq!(
        merge_named_attributes([], &[&object], 2, &mut exact)
            .unwrap()
            .get("long_key"),
        Some(&3)
    );
    assert_eq!(exact.steps, 0);
}

#[test]
fn positional_projection_rejection_retains_original_location() {
    let outer = properties(&[("x", 1)]);
    let inner = Object::from_entries(vec![
        Entry::Property {
            key: "y".into(),
            value: 2,
        },
        Entry::Value(3),
    ]);
    assert!(matches!(
        merge_named_attributes([], &[&outer, &inner], 5, &mut Budget::default()),
        Err(MergeError::PositionalEntry {
            object: 1,
            entry: 1
        })
    ));
    assert_eq!(inner.entries().len(), 2);
}

#[test]
fn empty_objects_still_spend_fuel_and_dumps_escape_keys() {
    let empty = Object::<i32>::from_entries(vec![]);
    let mut budget = Budget {
        steps: 2,
        ..Budget::default()
    };
    assert!(matches!(
        merge_named_attributes([], &[&empty, &empty], 0, &mut budget),
        Err(MergeError::BudgetExhausted)
    ));
    let object = properties(&[("quote\"\\\n雪", 7)]);
    let merged = merge_named_attributes([], &[&object], 1, &mut Budget::default()).unwrap();
    assert_eq!(
        merged.to_string(),
        "(attributes (property (key \"quote\\\"\\\\\\n雪\") (value 7)))"
    );
    assert_eq!(merged.to_string(), merged.to_string());
}
