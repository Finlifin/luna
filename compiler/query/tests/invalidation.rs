use query::{QueryCache, QueryDesc, QueryEngine, QueryResult, force_get_with, try_get_with};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
const QUERY: QueryDesc = QueryDesc::new("value");

#[test]
fn cached_read_edges_invalidate_transitively_without_display_key_collisions() {
    let engine = QueryEngine::new();
    let cache = QueryCache::new();
    let input = Cell::new(10);
    let runs = Cell::new(0);
    let leaf = || force_get_with(&engine, &cache, QUERY, 1, |_| "same".into(), || input.get());
    // The first parent observes a cache hit, which must still add its edge.
    assert_eq!(leaf(), 10);
    let parent = || {
        force_get_with(
            &engine,
            &cache,
            QUERY,
            2,
            |_| "same".into(),
            || {
                runs.set(runs.get() + 1);
                leaf() + 1
            },
        )
    };
    let root = || {
        force_get_with(
            &engine,
            &cache,
            QUERY,
            3,
            |_| "same".into(),
            || parent() + 1,
        )
    };
    let other = || force_get_with(&engine, &cache, QUERY, 4, |_| "same".into(), || 99);
    assert_eq!(root(), 12);
    assert_eq!(other(), 99);
    engine.update_input(&cache, &1, || input.set(20)).unwrap();
    assert_eq!(root(), 22);
    assert_eq!(root(), 22);
    assert_eq!(other(), 99);
    assert_eq!(runs.get(), 2);
    let dump = engine.dependency_dump();
    assert!(dump.starts_with("(query-dependencies"));
    assert_eq!(dump.matches("(node ").count(), 4);
    assert_eq!(dump, engine.dependency_dump());
}

#[test]
fn recomputation_replaces_obsolete_dependency_edges() {
    let engine = QueryEngine::new();
    let leaves = QueryCache::new();
    let roots = QueryCache::new();
    let selected = Cell::new(1);
    let runs = Cell::new(0);
    let root = || {
        force_get_with(
            &engine,
            &roots,
            QUERY,
            1,
            |_| "same".into(),
            || {
                runs.set(runs.get() + 1);
                force_get_with(
                    &engine,
                    &leaves,
                    QUERY,
                    selected.get(),
                    |_| "same".into(),
                    || selected.get(),
                )
            },
        )
    };
    assert_eq!(root(), 1);
    engine.update_input(&roots, &1, || selected.set(2)).unwrap();
    assert_eq!(root(), 2);
    engine.update_input(&leaves, &1, || {}).unwrap();
    assert_eq!(root(), 2);
    assert_eq!(runs.get(), 2);
    engine.update_input(&leaves, &2, || {}).unwrap();
    assert_eq!(root(), 2);
    assert_eq!(runs.get(), 3);
}

#[test]
fn observed_dependencies_recover_poisoned_queries_after_input_change() {
    let engine = QueryEngine::new();
    let cache = QueryCache::new();
    let input = Cell::new(0);
    let root = || {
        force_get_with(
            &engine,
            &cache,
            QUERY,
            2,
            |k| k.to_string(),
            || {
                let value =
                    force_get_with(&engine, &cache, QUERY, 1, |k| k.to_string(), || input.get());
                assert!(value > 0, "invalid input");
                value
            },
        )
    };
    assert!(catch_unwind(AssertUnwindSafe(&root)).is_err());
    assert!(matches!(
        try_get_with(&engine, &cache, QUERY, 2, |k| k.to_string(), || 0),
        QueryResult::Poisoned
    ));
    engine.update_input(&cache, &1, || input.set(7)).unwrap();
    assert_eq!(root(), 7);
    assert!(engine.stack().is_empty());
}

#[test]
fn input_updates_are_rejected_during_queries_and_dirty_before_panicking_updates() {
    let engine = QueryEngine::new();
    let cache = QueryCache::new();
    let input = Cell::new(1);
    assert_eq!(
        force_get_with(
            &engine,
            &cache,
            QUERY,
            1,
            |k| k.to_string(),
            || {
                assert!(engine.update_input(&cache, &1, || input.set(99)).is_err());
                input.get()
            }
        ),
        1
    );
    assert_eq!(input.get(), 1);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            engine
                .update_input(&cache, &1, || {
                    input.set(2);
                    panic!("update panic")
                })
                .unwrap();
        }))
        .is_err()
    );
    assert_eq!(
        force_get_with(&engine, &cache, QUERY, 1, |k| k.to_string(), || input.get()),
        2
    );
}

#[test]
fn independent_cache_clones_do_not_inherit_untracked_results() {
    let engine = QueryEngine::new();
    let cache = QueryCache::new();
    assert_eq!(
        force_get_with(&engine, &cache, QUERY, 1, |_| "\"line\n".into(), || 7),
        7
    );
    let cloned = cache.clone();
    assert!(cloned.is_empty());
    assert_eq!(
        force_get_with(&engine, &cloned, QUERY, 1, |_| "\"line\n".into(), || 9),
        9
    );
    assert!(engine.dependency_dump().contains("\\\"line\\n"));
}

#[test]
fn reads_inside_input_updates_cannot_cache_intermediate_state() {
    let engine = QueryEngine::new();
    let cache = QueryCache::new();
    let input = Cell::new(1);
    let read = || force_get_with(&engine, &cache, QUERY, 1, |k| k.to_string(), || input.get());
    assert_eq!(read(), 1);
    engine
        .update_input(&cache, &1, || {
            assert_eq!(read(), 1);
            input.set(2);
        })
        .unwrap();
    assert_eq!(read(), 2);
}
