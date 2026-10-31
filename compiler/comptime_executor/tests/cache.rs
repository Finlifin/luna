use comptime_executor::{Cache, Context, Error, KeyProtocol, QueryId, Scope};

fn scope() -> Scope {
    Scope {
        definitions: 1,
        protocols: 2,
        target: 3,
    }
}
fn query(declaration: u32) -> QueryId {
    QueryId {
        package: 7,
        declaration,
        operation: 0,
    }
}
#[derive(Default)]
struct FoldCase {
    fail_hash: bool,
    fail_eq: bool,
    comparisons: usize,
}
impl KeyProtocol<String> for FoldCase {
    type Error = &'static str;
    fn hash(&mut self, value: &String, context: &mut Context) -> Result<u64, Error<Self::Error>> {
        context.spend(value.len())?;
        if self.fail_hash {
            return Err(Error::Protocol("hash failed"));
        }
        Ok(0) // Deliberate collision: the table must actually run source equality.
    }
    fn equivalent(
        &mut self,
        a: &String,
        b: &String,
        context: &mut Context,
    ) -> Result<bool, Error<Self::Error>> {
        context.spend(a.len() + b.len())?;
        self.comparisons += 1;
        if self.fail_eq {
            return Err(Error::Protocol("eq failed"));
        }
        Ok(a.eq_ignore_ascii_case(b))
    }
}

#[test]
fn collisions_use_semantic_equality_and_preserve_order_and_query_identity() {
    let mut cache = Cache::new(scope(), 20);
    let mut context = Context::new(10_000);
    let mut protocol = FoldCase::default();
    let mut executions = 0;
    for (id, inputs, reused) in [
        (query(1), vec!["Alpha", "Beta"], false),
        (query(1), vec!["ALPHA", "beta"], true),
        (query(1), vec!["Beta", "Alpha"], false),
        (query(1), vec!["Other", "Beta"], false),
        (query(2), vec!["Alpha", "Beta"], false),
        (
            QueryId {
                package: 8,
                ..query(1)
            },
            vec!["Alpha", "Beta"],
            false,
        ),
        (
            QueryId {
                operation: 1,
                ..query(1)
            },
            vec!["Alpha", "Beta"],
            false,
        ),
        (query(1), vec!["Alpha"], false),
    ] {
        let expected = inputs.join("/").to_lowercase();
        let result = cache
            .query_pure(
                id,
                inputs.into_iter().map(str::to_owned).collect(),
                &mut context,
                &mut protocol,
                |inputs, _| {
                    executions += 1;
                    Ok(inputs.join("/").to_lowercase())
                },
            )
            .unwrap();
        assert_eq!(*result.value, expected);
        assert_eq!(result.reused, reused);
    }
    assert_eq!(executions, 7);
    assert_eq!(cache.len(), 7);
    assert!(protocol.comparisons >= 3);
}

#[test]
fn failures_do_not_publish_entries_or_poison_later_key_formation() {
    let mut cache = Cache::new(scope(), 2);
    let mut context = Context::new(10_000);
    let mut protocol = FoldCase {
        fail_hash: true,
        ..Default::default()
    };
    let evaluate = |_: &[String], _: &mut Context| Ok(42);
    assert!(matches!(
        cache.query_pure(
            query(1),
            vec!["a".into()],
            &mut context,
            &mut protocol,
            evaluate
        ),
        Err(Error::Protocol("hash failed"))
    ));
    assert!(cache.is_empty());
    protocol.fail_hash = false;
    assert!(matches!(
        cache.query_pure(
            query(1),
            vec!["a".into()],
            &mut context,
            &mut protocol,
            |_, _| Err::<i32, _>("evaluation failed")
        ),
        Err(Error::Evaluation(_))
    ));
    assert!(cache.is_empty());
    cache
        .query_pure(
            query(1),
            vec!["a".into()],
            &mut context,
            &mut protocol,
            evaluate,
        )
        .unwrap();
    protocol.fail_eq = true;
    assert!(matches!(
        cache.query_pure(
            query(1),
            vec!["b".into()],
            &mut context,
            &mut protocol,
            evaluate
        ),
        Err(Error::Protocol("eq failed"))
    ));
    assert_eq!(cache.len(), 1);
    protocol.fail_eq = false;
    assert!(
        cache
            .query_pure(
                query(1),
                vec!["A".into()],
                &mut context,
                &mut protocol,
                |_, _| panic!("hit evaluated query")
            )
            .unwrap()
            .reused
    );
    assert!(matches!(
        cache.query_pure(
            query(1),
            vec!["b".into()],
            &mut Context::new(1),
            &mut protocol,
            evaluate
        ),
        Err(Error::BudgetExhausted)
    ));
    cache
        .query_pure(
            query(1),
            vec!["b".into()],
            &mut context,
            &mut protocol,
            evaluate,
        )
        .unwrap();
    assert!(matches!(
        cache.query_pure(
            query(1),
            vec!["c".into()],
            &mut context,
            &mut protocol,
            |_, _| unreachable!()
        ),
        Err(Error::CapacityExceeded)
    ));
}

#[test]
fn dependency_revisions_invalidate_results_but_unchanged_scope_keeps_them() {
    let mut cache = Cache::new(scope(), 1);
    let mut context = Context::new(1000);
    let mut protocol = FoldCase::default();
    let mut executions = 0;
    for (scope, reused) in [
        (scope(), false),
        (scope(), true),
        (
            Scope {
                definitions: 2,
                ..scope()
            },
            false,
        ),
        (
            Scope {
                definitions: 2,
                protocols: 3,
                ..scope()
            },
            false,
        ),
        (
            Scope {
                definitions: 2,
                protocols: 3,
                target: 4,
            },
            false,
        ),
    ] {
        cache.set_scope(scope);
        let result = cache
            .query_pure(
                query(1),
                vec!["a".into()],
                &mut context,
                &mut protocol,
                |_, _| {
                    executions += 1;
                    Ok(executions)
                },
            )
            .unwrap();
        assert_eq!(result.reused, reused);
    }
    assert_eq!(executions, 4);
}

#[test]
fn hash_reentry_is_rejected_and_guard_restores_after_error() {
    struct Reentrant {
        recurse: bool,
    }
    impl KeyProtocol<String> for Reentrant {
        type Error = &'static str;
        fn hash(
            &mut self,
            value: &String,
            context: &mut Context,
        ) -> Result<u64, Error<Self::Error>> {
            if self.recurse {
                let mut other = Cache::<String, ()>::new(scope(), 1);
                other.query_pure(
                    query(2),
                    vec![value.clone()],
                    context,
                    &mut FoldCase::default(),
                    |_, _| Ok(()),
                )?;
            }
            Ok(0)
        }
        fn equivalent(
            &mut self,
            a: &String,
            b: &String,
            _: &mut Context,
        ) -> Result<bool, Error<Self::Error>> {
            Ok(a == b)
        }
    }
    let mut protocol = Reentrant { recurse: true };
    let mut cache = Cache::new(scope(), 1);
    let mut context = Context::new(100);
    assert!(matches!(
        cache.query_pure(
            query(1),
            vec!["a".into()],
            &mut context,
            &mut protocol,
            |_, _| Ok(())
        ),
        Err(Error::ReentrantKeyFormation)
    ));
    assert!(cache.is_empty());
    protocol.recurse = false;
    assert!(
        cache
            .query_pure(
                query(1),
                vec!["a".into()],
                &mut context,
                &mut protocol,
                |_, _| Ok(())
            )
            .is_ok()
    );
}

#[test]
fn comparison_budget_failure_keeps_cached_result_and_allows_retry() {
    let mut cache = Cache::new(scope(), 1);
    let mut protocol = FoldCase::default();
    let mut context = Context::new(100);
    cache
        .query_pure(
            query(1),
            vec!["a".into()],
            &mut context,
            &mut protocol,
            |_, _| Ok(42),
        )
        .unwrap();
    let result = cache.query_pure(
        query(1),
        vec!["A".into()],
        &mut Context::new(5),
        &mut protocol,
        |_, _| panic!("must not evaluate during failed comparison"),
    );
    assert!(matches!(result, Err(Error::BudgetExhausted)));
    assert_eq!(cache.len(), 1);
    let result = cache
        .query_pure(
            query(1),
            vec!["A".into()],
            &mut context,
            &mut protocol,
            |_, _| panic!("full cache must still permit a hit"),
        )
        .unwrap();
    assert!(result.reused);
    assert_eq!(*result.value, 42);
}

#[test]
fn ordinary_nested_evaluation_is_allowed_after_key_formation_finishes() {
    let mut outer = Cache::new(scope(), 2);
    let mut inner = Cache::new(scope(), 2);
    let mut context = Context::new(1000);
    let result = outer
        .query_pure(
            query(1),
            vec!["a".into()],
            &mut context,
            &mut FoldCase::default(),
            |inputs, context| {
                let nested = inner
                    .query_pure(
                        query(2),
                        inputs.to_vec(),
                        context,
                        &mut FoldCase::default(),
                        |_, _| Ok(42),
                    )
                    .unwrap();
                Ok(*nested.value)
            },
        )
        .unwrap();
    assert_eq!(*result.value, 42);
    assert_eq!(outer.len(), 1);
    assert_eq!(inner.len(), 1);
    assert_eq!(
        query(1).to_string(),
        "(comptime-query (package 7) (declaration 1) (operation 0))"
    );
    assert!(scope().to_string().starts_with("(query-scope "));
}

#[test]
fn successful_unadmitted_results_return_without_becoming_cache_entries() {
    let mut cache = Cache::new(scope(), 10);
    let mut context = Context::new(10_000);
    let mut protocol = FoldCase::default();
    let mut executions = 0;
    for publish in [false, false, true, true] {
        let before = executions;
        let outcome = cache
            .query_pure_admitted(
                query(1),
                vec!["Alpha".to_string()],
                &mut context,
                &mut protocol,
                |inputs, _| {
                    executions += 1;
                    Ok((inputs[0].to_lowercase(), publish))
                },
            )
            .unwrap();
        assert_eq!(*outcome.value, "alpha");
        assert_eq!(outcome.reused, before == 3);
        assert_eq!(cache.len(), usize::from(publish));
    }
    assert_eq!(executions, 3);
}

#[test]
fn admitted_queries_continue_uncached_at_capacity_without_losing_hits() {
    let mut cache = Cache::new(scope(), 1);
    let mut context = Context::new(10_000);
    let mut protocol = FoldCase::default();
    let mut executions = 0;
    for (input, reused) in [("a", false), ("b", false), ("b", false), ("a", true)] {
        let output = cache
            .query_pure_admitted(
                query(1),
                vec![input.into()],
                &mut context,
                &mut protocol,
                |inputs, _| {
                    executions += 1;
                    Ok((inputs[0].clone(), true))
                },
            )
            .unwrap();
        assert_eq!(output.value.as_str(), input);
        assert_eq!(output.reused, reused);
        assert_eq!(cache.len(), 1);
    }
    assert_eq!(executions, 3);
}
