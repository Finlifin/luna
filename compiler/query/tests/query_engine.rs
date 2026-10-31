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

    let r1 = force_get_with(
        &engine,
        &cache,
        A,
        1,
        |k| k.to_string(),
        || {
            hits.set(hits.get() + 1);
            42
        },
    );
    let r2 = force_get_with(
        &engine,
        &cache,
        A,
        1,
        |k| k.to_string(),
        || {
            hits.set(hits.get() + 1);
            42
        },
    );
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
                || match try_get_with(&engine, &cache_a, A, "a", |k| (*k).to_string(), || 0) {
                    QueryResult::Cycle(err) => {
                        cycle_frames.set(Some(
                            err.frames.iter().map(|f| f.desc.name).collect::<Vec<_>>(),
                        ));
                        0
                    }
                    other => panic!("expected Cycle, got {other:?}"),
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
        force_get_with(
            &engine,
            &cache,
            A,
            7,
            |k| k.to_string(),
            || {
                panic!("boom");
            },
        );
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
    force_get_with(
        &engine,
        &cache,
        A,
        1,
        |k| k.to_string(),
        || {
            h.set(h.get() + 1);
            10
        },
    );
    let h = hits.clone();
    force_get_with(
        &engine,
        &cache,
        A,
        2,
        |k| k.to_string(),
        || {
            h.set(h.get() + 1);
            20
        },
    );
    assert_eq!(hits.get(), 2);
    assert_eq!(
        force_get_with(&engine, &cache, A, 1, |k| k.to_string(), || 99),
        10
    );
}

#[test]
fn shared_engine_serializes_concurrent_requests_and_computes_once() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    let engine = Arc::new(QueryEngine::new());
    let cache = Arc::new(QueryCache::new());
    let ready = Arc::new(Barrier::new(8));
    let calls = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for _ in 0..8 {
            let (engine, cache, ready, calls) =
                (engine.clone(), cache.clone(), ready.clone(), calls.clone());
            threads.push(scope.spawn(move || {
                ready.wait();
                force_get_with(
                    &engine,
                    &cache,
                    A,
                    1,
                    |key| key.to_string(),
                    || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        // Reentrant evaluation must not deadlock on the engine gate.
                        force_get_with(&engine, &cache, B, 2, |key| key.to_string(), || 41) + 1
                    },
                )
            }));
        }
        for thread in threads {
            assert_eq!(thread.join().unwrap(), 42);
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(engine.stack().is_empty());
}

#[test]
fn shared_engine_releases_ownership_after_provider_panic() {
    let engine = QueryEngine::new();
    let cache = QueryCache::new();
    std::thread::scope(|scope| {
        assert!(
            scope
                .spawn(|| {
                    force_get_with(
                        &engine,
                        &cache,
                        A,
                        1,
                        |key| key.to_string(),
                        || -> u32 { panic!("provider panic") },
                    )
                })
                .join()
                .is_err()
        );
        scope
            .spawn(|| {
                assert!(matches!(
                    try_get_with(&engine, &cache, A, 1, |key| key.to_string(), || 0),
                    QueryResult::Poisoned
                ));
                assert_eq!(
                    force_get_with(&engine, &cache, A, 2, |key| key.to_string(), || 42),
                    42
                );
            })
            .join()
            .unwrap();
    });
    assert!(engine.stack().is_empty());
}

#[test]
fn start_hook_panic_poisoning_keeps_the_stack_balanced() {
    let engine = QueryEngine::with_hooks(query::QueryHooks {
        on_query_start: std::sync::Arc::new(|_| panic!("hook panic")),
        ..Default::default()
    });
    let cache = QueryCache::new();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            force_get_with(&engine, &cache, A, 1, |key| key.to_string(), || 42)
        }))
        .is_err()
    );
    assert!(engine.stack().is_empty());
    assert!(matches!(cache.get(&1), Some(CacheState::Poisoned)));
}

#[test]
fn start_hook_reentry_observes_claimed_slot_and_cycle_stack() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let cache = Arc::new(QueryCache::new());
    let observed = Arc::new(AtomicBool::new(false));
    let engine: Arc<QueryEngine> = Arc::new_cyclic(|weak: &std::sync::Weak<QueryEngine>| {
        let weak = weak.clone();
        let cache = cache.clone();
        let observed = observed.clone();
        QueryEngine::with_hooks(query::QueryHooks {
            on_query_start: Arc::new(move |_| {
                let engine = weak.upgrade().unwrap();
                let result = try_get_with(&engine, &cache, A, 1, |key| key.to_string(), || 0);
                let QueryResult::Cycle(cycle) = result else {
                    panic!("expected hook reentry cycle")
                };
                assert_eq!(cycle.frames.len(), 2);
                observed.store(true, Ordering::SeqCst);
            }),
            ..Default::default()
        })
    });
    assert_eq!(
        force_get_with(&engine, &cache, A, 1, |key| key.to_string(), || 42),
        42
    );
    assert!(observed.load(Ordering::SeqCst));
    assert!(engine.stack().is_empty());
}

#[test]
fn competing_query_waits_for_active_provider_instead_of_reporting_a_cycle() {
    use std::sync::mpsc;
    use std::time::Duration;
    let engine = QueryEngine::new();
    let cache = QueryCache::new();
    let (entered, started) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (ready, waiting) = mpsc::channel();
    let (result, received) = mpsc::channel();
    std::thread::scope(|scope| {
        let engine_ref = &engine;
        let cache_ref = &cache;
        let producer = scope.spawn(move || {
            force_get_with(
                engine_ref,
                cache_ref,
                A,
                1,
                |key| key.to_string(),
                || {
                    entered.send(()).unwrap();
                    resume.recv().unwrap();
                    42
                },
            )
        });
        started.recv().unwrap();
        let consumer = scope.spawn(|| {
            ready.send(()).unwrap();
            result
                .send(try_get_with(
                    &engine,
                    &cache,
                    A,
                    1,
                    |key| key.to_string(),
                    || 0,
                ))
                .unwrap();
        });
        waiting.recv().unwrap();
        let premature = received.recv_timeout(Duration::from_millis(50));
        // Always release the producer before assertions, including a failing run.
        release.send(()).unwrap();
        assert_eq!(producer.join().unwrap(), 42);
        consumer.join().unwrap();
        assert!(matches!(premature, Err(mpsc::RecvTimeoutError::Timeout)));
        assert!(matches!(received.recv().unwrap(), QueryResult::Ok(42)));
    });
}
