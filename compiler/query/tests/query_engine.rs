//! Unit tests for the query infrastructure.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use query::{
    CacheState, QueryCache, QueryDesc, QueryEngine, QueryResult, force_get_with, try_get_with,
};

const A: QueryDesc = QueryDesc::new("query_a");
const B: QueryDesc = QueryDesc::new("query_b");

#[test]
fn memoises_compute() {
    let engine = QueryEngine::new();
    let cache = QueryCache::<u32, u32>::new();
    let hits = Cell::new(0);

    let r1 = force_get_with(&engine, &cache, A, 1, |k| k.to_string(), || {
        hits.set(hits.get() + 1);
        42
    });
    let r2 = force_get_with(&engine, &cache, A, 1, |k| k.to_string(), || {
        hits.set(hits.get() + 1);
        42
    });
    assert_eq!(r1, 42);
    assert_eq!(r2, 42);
    assert_eq!(hits.get(), 1);
}

#[test]
fn detects_cycle() {
    let engine = QueryEngine::new();
    let cache_a = QueryCache::<&'static str, i32>::new();
    let cache_b = QueryCache::<&'static str, i32>::new();
    let cycle_frames = Cell::new(None);

    let result = try_get_with(
        &engine,
        &cache_a,
        A,
        "a",
        |k| (*k).to_string(),
        || {
            let inner = try_get_with(
                &engine,
                &cache_b,
                B,
                "b",
                |k| (*k).to_string(),
                || {
                    match try_get_with(
                        &engine,
                        &cache_a,
                        A,
                        "a",
                        |k| (*k).to_string(),
                        || 0,
                    ) {
                        QueryResult::Cycle(err) => {
                            cycle_frames.set(Some(
                                err.frames
                                    .iter()
                                    .map(|f| f.desc.name)
                                    .collect::<Vec<_>>(),
                            ));
                            0
                        }
                        other => panic!("expected Cycle, got {other:?}"),
                    }
                },
            );
            match inner {
                QueryResult::Ok(v) => v,
                other => panic!("expected Ok from b, got {other:?}"),
            }
        },
    );

    assert!(matches!(result, QueryResult::Ok(0)));
    assert_eq!(
        cycle_frames.take().as_deref(),
        Some(["query_a", "query_b", "query_a"].as_slice())
    );
}

#[test]
fn poison_on_panic() {
    let engine = QueryEngine::new();
    let cache = QueryCache::<u32, u32>::new();

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        force_get_with(&engine, &cache, A, 7, |k| k.to_string(), || {
            panic!("boom");
        });
    }));
    assert!(panicked.is_err());

    match cache.get(&7) {
        Some(CacheState::Poisoned) => {}
        other => panic!("expected Poisoned, got {other:?}"),
    }

    match try_get_with(&engine, &cache, A, 7, |k| k.to_string(), || 1) {
        QueryResult::Poisoned => {}
        other => panic!("expected Poisoned result, got {other:?}"),
    }
}

#[test]
fn different_keys_independent() {
    let engine = QueryEngine::new();
    let cache = QueryCache::<u32, u32>::new();
    let hits = Rc::new(Cell::new(0));

    let h = hits.clone();
    force_get_with(&engine, &cache, A, 1, |k| k.to_string(), || {
        h.set(h.get() + 1);
        10
    });
    let h = hits.clone();
    force_get_with(&engine, &cache, A, 2, |k| k.to_string(), || {
        h.set(h.get() + 1);
        20
    });
    assert_eq!(hits.get(), 2);
    assert_eq!(
        force_get_with(&engine, &cache, A, 1, |k| k.to_string(), || 99),
        10
    );
}
