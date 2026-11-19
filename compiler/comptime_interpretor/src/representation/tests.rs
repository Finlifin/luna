use super::*;

#[test]
fn text_clones_share_bytes_but_keep_expanded_size_limits() {
    let mut costs = Vec::new();
    for len in [32, 4096] {
        let text: std::sync::Arc<str> = "x".repeat(len).into();
        let value = Value::Tuple(vec![Value::String(text.clone()), Value::Symbol(text)]);
        costs.push(value.immutable_clone_work().unwrap());
        assert!(value.immutable_copy_work().unwrap() >= len * 2);
        let copy = value.clone();
        let (Value::Tuple(original), Value::Tuple(copied)) = (&value, &copy) else {
            unreachable!()
        };
        let (Value::String(original), Value::String(copied)) = (&original[0], &copied[0]) else {
            unreachable!()
        };
        assert!(std::sync::Arc::ptr_eq(original, copied));
    }
    assert_eq!(costs[0], costs[1]);
    let oversized = Value::String("x".repeat(100_000).into());
    assert!(oversized.immutable_clone_work().is_none());
    assert!(
        Value::String("small".into())
            .immutable_clone_work()
            .is_some()
    );
}

fn list(len: usize) -> Value {
    Value::List {
        ty: Term::Primitive(Primitive::Bool),
        values: vec![Value::Bool(true); len].into(),
    }
}

#[test]
fn shared_payload_copy_cost_is_separate_from_expanded_size() {
    let mut costs = Vec::new();
    for len in [32, 4096] {
        let value = Value::Tuple(vec![Value::Bool(true), list(len)]);
        let cold = value.immutable_clone_work().unwrap();
        let warm = value.immutable_clone_work().unwrap();
        assert!(cold >= len);
        assert!(value.immutable_copy_work().unwrap() >= len);
        assert!(warm < cold);
        costs.push(warm);
        let mut snapshot = value.clone();
        let Value::Tuple(fields) = &mut snapshot else {
            unreachable!()
        };
        let Value::List { values, .. } = &mut fields[1] else {
            unreachable!()
        };
        values[0] = Value::Bool(false);
        assert_ne!(snapshot, value);
        assert_eq!(value.immutable_clone_work(), Some(warm));
        let Value::Tuple(fields) = &mut snapshot else {
            unreachable!()
        };
        let Value::List { values, .. } = &mut fields[1] else {
            unreachable!()
        };
        values[0] = Value::Storage(Storage::new(Value::Bool(true)));
        assert_eq!(snapshot.immutable_clone_work(), None);
        assert_eq!(value.immutable_clone_work(), Some(warm));
    }
    assert_eq!(costs[0], costs[1]);
}

#[test]
fn shared_payloads_keep_size_and_depth_rejection() {
    let left = list(60_000);
    left.immutable_clone_work().unwrap();
    let Value::List { mut values, ty } = left else {
        unreachable!()
    };
    let Value::List { values: right, .. } = list(60_000) else {
        unreachable!()
    };
    let warmed = Value::List {
        ty: ty.clone(),
        values: right,
    };
    warmed.immutable_clone_work().unwrap();
    let Value::List { values: right, .. } = warmed else {
        unreachable!()
    };
    values.append_payload(right);
    assert!(Value::List { ty, values }.immutable_clone_work().is_none());
    let mut nested = list(1);
    nested.immutable_clone_work().unwrap();
    for _ in 0..128 {
        nested = Value::Tuple(vec![nested]);
    }
    assert!(nested.immutable_clone_work().is_none());
    assert!(list(1).immutable_clone_work().is_some());
}

#[test]
fn failed_scans_pay_for_each_visited_element_before_a_summary_can_be_published() {
    let value = list(16);
    let Value::List { values, .. } = &value else {
        unreachable!()
    };
    for initial in [0, 1, 5, 17] {
        let mut fuel = initial;
        assert_eq!(
            value.inspect_immutable(&mut fuel),
            Err(ErrorKind::StepsExhausted)
        );
        assert_eq!(fuel, 0);
        assert!(values.summary().is_none());
    }
    let mut fuel = 18;
    let size = value.inspect_immutable(&mut fuel).unwrap();
    assert_eq!(fuel, 0);
    assert_eq!(
        size,
        ImmutableSize {
            expanded: 18,
            owned: 2
        }
    );
    assert!(values.summary().is_some());
    let mut warm = 2;
    assert_eq!(value.inspect_immutable(&mut warm).unwrap(), size);
    assert_eq!(warm, 0);
}

#[test]
fn host_and_list_boundaries_report_the_representation_failure() {
    let source = SourceRange { start: 3, end: 7 };
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ty = Term::Primitive(Primitive::Bool);
    let packet = |payload| Value::List {
        ty: ty.clone(),
        values: vec![payload].into(),
    };
    let mut nested = Value::Bool(true);
    for _ in 0..127 {
        nested = Value::Tuple(vec![nested]);
    }
    for (payload, error) in [
        (
            Value::String("x".repeat(MAX_SIZE).into()),
            ErrorKind::RepresentationSizeLimit,
        ),
        (nested, ErrorKind::DepthExhausted),
        (Value::Type(Term::Bool(false)), ErrorKind::Type),
        (
            Value::Storage(Storage::new(Value::Unit)),
            ErrorKind::Unsupported,
        ),
    ] {
        let value = packet(payload);
        vm.budget.steps = 1_000_000;
        let prior = vm.remaining_steps();
        assert_eq!(
            vm.validate_immutable_input(&value, &ty).unwrap_err().kind,
            error
        );
        assert!(vm.remaining_steps() < prior);
        let empty = Value::List {
            ty: ty.clone(),
            values: vec![].into(),
        };
        assert_eq!(
            vm.complete_list_concat(empty, value, 0, 1, source)
                .unwrap_err(),
            Error {
                kind: error,
                source: Some(source)
            }
        );
    }
}
