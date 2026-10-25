use super::*;

fn expr(kind: K, ty: Primitive) -> Expr {
    Expr {
        kind,
        ty: Term::Primitive(ty),
        source: SourceRange { start: 1, end: 2 },
    }
}
fn spine(mut leaf: Expr, count: usize) -> Expr {
    for index in 0..count {
        leaf = Expr {
            source: SourceRange {
                start: index as u32 + 3,
                end: index as u32 + 4,
            },
            ty: leaf.ty.clone(),
            kind: K::Not(Box::new(leaf)),
        };
    }
    leaf
}

#[test]
fn deep_boolean_spines_charge_every_node_and_recover_after_failure() {
    let package = Package::default();
    for count in [63, 64, 96] {
        let expression = spine(
            expr(K::Literal(Literal::Bool(true)), Primitive::Bool),
            count,
        );
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 1000,
                depth: 2,
            },
        )
        .unwrap();
        let before = vm.remaining_steps();
        assert!(
            matches!(vm.eval(&expression, &mut Environment::new()).unwrap(), Flow::Value(Value::Bool(value)) if value == (count % 2 == 0))
        );
        assert_eq!(before - vm.remaining_steps(), (count + 1) as u64);
        assert_eq!(vm.depth, 0);

        let mut limited = Interpreter::new(
            &package,
            Budget {
                steps: 10,
                depth: 2,
            },
        )
        .unwrap();
        let error = limited
            .eval(&expression, &mut Environment::new())
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(limited.depth, 0);
        // Fresh invocation state can evaluate the same immutable tree.
        assert!(vm.eval(&expression, &mut Environment::new()).is_ok());
    }
}

#[test]
fn bit_spines_validate_each_intermediate_domain() {
    let package = Package::default();
    let leaf = expr(
        K::Literal(Literal::BitVec {
            width: 8,
            words: vec![0x55],
        }),
        Primitive::BitVec(8),
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let original = vm.eval(&leaf, &mut Environment::new()).unwrap();
    let Flow::Value(original) = original else {
        panic!("expected bits")
    };
    let mut expression = spine(leaf, 64);
    assert!(
        matches!(vm.eval(&expression, &mut Environment::new()).unwrap(), Flow::Value(value) if value == original)
    );
    let K::Not(inner) = &mut expression.kind else {
        unreachable!()
    };
    inner.ty = Term::Primitive(Primitive::BitVec(7));
    let source = inner.source;
    let error = vm.eval(&expression, &mut Environment::new()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Type);
    assert_eq!(error.source, Some(source));
    assert_eq!(vm.depth, 0);
}

#[test]
fn spines_propagate_return_without_negating_it() {
    let package = Package::default();
    let leaf = expr(
        K::Return(Some(Box::new(expr(
            K::Literal(Literal::Bool(true)),
            Primitive::Bool,
        )))),
        Primitive::Never,
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert!(matches!(
        vm.eval(&spine(leaf, 64), &mut Environment::new()).unwrap(),
        Flow::Return(Value::Bool(true))
    ));
    assert_eq!(vm.depth, 0);
}
